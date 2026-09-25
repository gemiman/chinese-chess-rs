/**
 * 前端交互冒烟测试 —— 用 CDP 驱动一个无头 Chrome，真点棋盘。
 *
 * 为什么需要它：棋盘渲染对不对，截图能看出来；但「点击选中 → 落点高亮 →
 * 点击落子 → 记谱更新」这条主交互，只有真的点一次才能确认。而这类 bug
 * （坐标换算差一格、标记层没渲染、点击没绑上）在静态截图里完全看不出来。
 *
 * # 用法
 *
 * ```bash
 * # 1. 起一个带调试端口的无头 Chrome
 * chrome --headless=new --remote-debugging-port=9222 \
 *        --user-data-dir=<临时目录> about:blank
 *
 * # 2. 起桥接服务（它会托管前端）
 * cargo run -p xq-bridge
 *
 * # 3. 跑本脚本
 * node scripts/dev/ui_probe.mjs [页面地址]
 * ```
 *
 * 退出码 0 = 全部通过；1 = 有用例失败。
 *
 * # 界面对不上时该改这里，不是改产品
 *
 * 这个脚本的价值全在于**它盯的是用户真看得见的东西**。所以选择器变了就该跟着改，
 * 不能为了让它好写而给产品加测试专用的 class 或 id —— 那样它测的就不再是
 * 用户看到的界面了。
 *
 * # 覆盖范围（对应三页结构）
 *
 *   设置页 → 开局
 *   对局页 → 棋盘渲染 / 选中 / 落点 / 落子 / 悔棋 / 记谱浮层 / 一屏不滚动
 *   终局   → 认输自动跳转
 *   分析页 → 结果 / 统计 / 逐手讲解 / 深度分析
 *   复盘   → 跳回对局页、复盘条翻手数
 *   人机   → 引擎自动应招 / 提示浮层给 3 条候选
 */

const CDP_PORT = Number(process.env.CDP_PORT ?? 9222)
const PAGE_URL = process.argv[2] ?? 'http://127.0.0.1:8848/'
const SHOT_PATH = process.env.SHOT_PATH ?? null
const SHOT_PATH_END = process.env.SHOT_PATH_END ?? null

const CDP = `http://127.0.0.1:${CDP_PORT}`
const ORIGIN = new URL(PAGE_URL).origin

/**
 * 视口必须比棋盘大，而且要**高度真实**。
 *
 * 无头 Chrome 的默认视口只有 762×484，而棋盘是按视口高度反推尺寸的 ——
 * 太矮的话棋盘会小得点不准，而且「一屏不滚动」这条根本测不出来。
 * 1280×900 是一台普通笔记本的窗口尺寸，正好是这条约束最该被验证的地方。
 */
const VIEWPORT = { width: 1280, height: 900 }

// 与 frontend/src/coords.ts 保持一致
const MARGIN = 40
const CELL = 60
const VIEW_W = 560
const VIEW_H = 620
const ROWS = 10

let failures = 0

function check(label, ok, detail = '') {
  if (ok) {
    console.log(`  ✓ ${label}`)
  } else {
    failures += 1
    console.log(`  ✗ ${label}${detail ? ` —— ${detail}` : ''}`)
  }
}

/** 把服务端局面复位到初始状态。
 *
 * 这一步不能省：桥接服务持有**全局单局状态**，上一次跑探针走的子会留到下一次，
 * 于是第二次运行时 h2 上已经没有棋子，点击自然「没反应」—— 一个纯粹的
 * 测试污染，却看起来像产品 bug。
 */
async function resetGame() {
  const res = await fetch(`${ORIGIN}/api/new`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: '{}',
  })
  if (!res.ok) {
    throw new Error(`复位局面失败（HTTP ${res.status}）。桥接服务起来了吗？`)
  }
}

/** 读一次服务端局面。界面上的东西要跟它对上，才说明前端没在自说自话。 */
async function apiState() {
  const res = await fetch(`${ORIGIN}/api/state`)
  if (!res.ok) throw new Error(`取局面失败（HTTP ${res.status}）`)
  return res.json()
}

/** 建一个新标签页并返回其 CDP 连接。 */
async function openTarget(url) {
  // 新版 Chrome 要求 PUT
  let res = await fetch(`${CDP}/json/new?${encodeURIComponent(url)}`, { method: 'PUT' })
  if (!res.ok) {
    res = await fetch(`${CDP}/json/new?${encodeURIComponent(url)}`)
  }
  if (!res.ok) {
    throw new Error(`无法新建标签页（HTTP ${res.status}）。Chrome 是否以 --remote-debugging-port 启动？`)
  }
  const target = await res.json()
  const socket = new WebSocket(target.webSocketDebuggerUrl)
  await new Promise((resolve, reject) => {
    socket.addEventListener('open', resolve, { once: true })
    socket.addEventListener('error', reject, { once: true })
  })

  let nextId = 0
  const pending = new Map()
  socket.addEventListener('message', (event) => {
    const message = JSON.parse(event.data)
    const resolve = pending.get(message.id)
    if (resolve) {
      pending.delete(message.id)
      resolve(message)
    }
  })

  function send(method, params = {}) {
    const id = (nextId += 1)
    return new Promise((resolve) => {
      pending.set(id, resolve)
      socket.send(JSON.stringify({ id, method, params }))
    })
  }

  return { targetId: target.id, send, close: () => socket.close() }
}

/** 在页面里求值，返回 JSON 化后的结果。 */
async function evaluate(client, expression) {
  const reply = await client.send('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true,
  })
  if (reply.result?.exceptionDetails) {
    throw new Error(`页面内求值抛错：${reply.result.exceptionDetails.text}`)
  }
  return reply.result?.result?.value
}

/** 把棋理坐标换算成页面上的像素位置。 */
function squareExpression(col, row) {
  return `(() => {
    const board = document.querySelector('.board');
    if (!board) return null;
    const r = board.getBoundingClientRect();
    const MARGIN = ${MARGIN}, CELL = ${CELL}, VW = ${VIEW_W}, VH = ${VIEW_H}, ROWS = ${ROWS};
    return {
      x: r.left + ((MARGIN + ${col} * CELL) / VW) * r.width,
      y: r.top + ((MARGIN + (ROWS - 1 - ${row}) * CELL) / VH) * r.height,
      width: r.width,
      height: r.height,
    };
  })()`
}

/**
 * 派发一次真实的鼠标点击。
 *
 * `mousePressed` 必须带 `buttons: 1` —— 不带的话 Chrome 不会把它当成一次按下，
 * 于是也就不会合成 `click` 事件（这也是踩过的坑）。
 */
async function clickAt(client, x, y) {
  await client.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, buttons: 0 })
  await client.send('Input.dispatchMouseEvent', {
    type: 'mousePressed',
    x,
    y,
    button: 'left',
    clickCount: 1,
    buttons: 1,
  })
  await client.send('Input.dispatchMouseEvent', {
    type: 'mouseReleased',
    x,
    y,
    button: 'left',
    clickCount: 1,
    buttons: 0,
  })
}

async function typeText(client, text) {
  for (const ch of text) {
    await client.send('Input.dispatchKeyEvent', { type: 'keyDown', text: ch })
    await client.send('Input.dispatchKeyEvent', { type: 'keyUp', text: ch })
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

/** 等某个选择器出现。找到返回 true，超时返回 false。 */
async function waitFor(client, selector, tries = 60) {
  for (let i = 0; i < tries; i += 1) {
    if (await evaluate(client, `!!document.querySelector(${JSON.stringify(selector)})`)) {
      return true
    }
    await sleep(100)
  }
  return false
}

/**
 * 点一个按钮，按**可见文字**找。
 *
 * 按文字找而不是按 class 找，是为了让选择器尽量贴近用户看到的东西：
 * class 会随重构改，文字改了才是真的改了产品。
 */
async function clickButton(client, scope, text) {
  return evaluate(
    client,
    `(() => {
      const root = document.querySelector(${JSON.stringify(scope)});
      if (!root) return 'no-scope';
      const btn = [...root.querySelectorAll('button')].find(
        (b) => b.textContent.trim() === ${JSON.stringify(text)},
      );
      if (!btn) return 'not-found';
      if (btn.disabled) return 'disabled';
      btn.click();
      return 'ok';
    })()`,
  )
}

/** 从设置页开局并进入对局页。 */
async function startGame(client) {
  const clicked = await clickButton(client, '.setup__start', '开始游戏')
  if (clicked !== 'ok') return clicked
  await waitFor(client, '.board')
  await sleep(300)
  return 'ok'
}

/** 走一步：先点起点再点终点。 */
async function playMove(client, fromCol, fromRow, toCol, toRow) {
  const a = await evaluate(client, squareExpression(fromCol, fromRow))
  await clickAt(client, a.x, a.y)
  await sleep(220)
  const b = await evaluate(client, squareExpression(toCol, toRow))
  await clickAt(client, b.x, b.y)
  await sleep(500)
}

async function main() {
  console.log(`ui_probe: 复位局面 → 打开 ${PAGE_URL}`)
  await resetGame()

  const client = await openTarget(PAGE_URL)
  await client.send('Page.enable')
  await client.send('Runtime.enable')
  // 视口要在量坐标之前设好，否则底部棋子落在视口外、点击派发不到
  await client.send('Emulation.setDeviceMetricsOverride', {
    ...VIEWPORT,
    deviceScaleFactor: 1,
    mobile: false,
  })

  // 等应用挂载并完成首次 /api/state 拉取（首屏是设置页）
  const setupReady = await waitFor(client, '.setup__start')
  if (!setupReady) {
    console.error('ui_probe: 设置页没渲染出来，后面都测不了')
    client.close()
    process.exit(1)
  }
  await sleep(200)

  console.log('\n[1] 设置页应给出全部开局选项')
  const setup = await evaluate(
    client,
    `({
      cards: [...document.querySelectorAll('.setup__grid .card__title')].map((t) => t.textContent.trim()),
      segments: ['对局模式', '限时档位', '走子动画速度'].map((name) => !!document.querySelector('[aria-label="' + name + '"]')),
      startDisabled: document.querySelector('.setup__start .btn')?.disabled ?? null,
      hash: location.hash,
    })`,
  )
  check('设置页在 #/setup', setup.hash === '#/setup', `实际 ${setup.hash}`)
  check(
    '列出了模式 / 限时 / 速度',
    ['对局模式', '限时', '走子速度'].every((t) => setup.cards.includes(t)),
    `实际 ${setup.cards.join('、')}`,
  )
  check('三组选择器都可操作', setup.segments.every(Boolean), `实际 ${JSON.stringify(setup.segments)}`)
  check('开始游戏按钮可用', setup.startDisabled === false, `disabled=${setup.startDisabled}`)
  check(
    '双人模式下不显示「我执哪一方」',
    !setup.cards.includes('我执哪一方'),
    '这条选项只对人机对战有意义',
  )

  console.log('\n[1b] 没有对局时深链到 #/play，应被送回设置页')
  const histBefore = await evaluate(client, `history.length`)
  await evaluate(client, `location.hash = '#/play'`)
  await sleep(600)
  const fallback = await evaluate(
    client,
    `({ hash: location.hash, len: history.length, setup: !!document.querySelector('.setup__start') })`,
  )
  check('被送回 #/setup', fallback.hash === '#/setup', `实际 ${fallback.hash}`)
  check('显示的仍是设置页', fallback.setup === true)
  // 兜底跳转用的是「替换」而不是「新开一条」。用新开的话这里会多出 2 条记录，
  // 而多出来的那条正是 #/play —— 按后退键会被送回 #/play、再被送回 #/setup，
  // 来回弹，永远退不出去。
  check(
    '兜底跳转没有多留一条历史记录',
    fallback.len <= histBefore + 1,
    `history ${histBefore} → ${fallback.len}`,
  )

  console.log('\n[2] 开始游戏 → 对局页，且**一屏放得下、不滚动**')
  const started = await startGame(client)
  check('点击开始游戏后进入对局页', started === 'ok', String(started))

  const board = await evaluate(
    client,
    `(() => {
      const b = document.querySelector('.board');
      if (!b) return null;
      const r = b.getBoundingClientRect();
      const bar = document.querySelector('.play__bar');
      return {
        width: Math.round(r.width),
        height: Math.round(r.height),
        pieces: document.querySelectorAll('.piece').length,
        boardImg: !!document.querySelector('.board__surface'),
        hash: location.hash,
        docScroll: document.documentElement.scrollHeight,
        winH: window.innerHeight,
        barBottom: bar ? Math.round(bar.getBoundingClientRect().bottom) : null,
      };
    })()`,
  )
  check('棋盘已渲染', board !== null)
  if (!board) {
    client.close()
    process.exit(1)
  }
  check('URL 变成 #/play', board.hash === '#/play', `实际 ${board.hash}`)
  check('棋子 32 枚', board.pieces === 32, `实际 ${board.pieces}`)
  check('棋盘底图已加载', board.boardImg)

  const expectedRatio = VIEW_H / VIEW_W
  const actualRatio = board.height / board.width
  check(
    `棋盘宽高比 ${expectedRatio.toFixed(3)}`,
    Math.abs(actualRatio - expectedRatio) < 0.01,
    `实际 ${actualRatio.toFixed(3)}（宽高比不对会导致棋子与格线错位）`,
  )
  // 这条是对局页的硬约束：要滚动才能看见自己的时间，这盘就没法下了
  check(
    '页面没有滚动条',
    board.docScroll <= board.winH,
    `文档高 ${board.docScroll} > 视口 ${board.winH}`,
  )
  check(
    '按钮行在视口之内',
    board.barBottom !== null && board.barBottom <= board.winH,
    `按钮行底部 ${board.barBottom} / 视口 ${board.winH}`,
  )

  console.log('\n[3] 点击红炮 h2 应选中并高亮 12 个落点')
  const h2 = await evaluate(client, squareExpression(7, 2))
  await clickAt(client, h2.x, h2.y)
  await sleep(250)

  const afterSelect = await evaluate(
    client,
    `({
      selected: document.querySelectorAll('.piece--selected').length,
      moves: document.querySelectorAll('.fx img[src*="fx-move"]').length,
      captures: document.querySelectorAll('.fx img[src*="fx-capture"]').length,
      selectRing: document.querySelectorAll('.fx img[src*="fx-select"]').length,
      chips: document.querySelectorAll('.hint-chip').length,
    })`,
  )
  check('恰好一枚棋子被选中', afterSelect.selected === 1, `实际 ${afterSelect.selected}`)
  check('选中环画出来了', afterSelect.selectRing === 1, `实际 ${afterSelect.selectRing}`)
  check(
    '棋盘上有 12 个落点标记',
    afterSelect.moves + afterSelect.captures === 12,
    `空格 ${afterSelect.moves} + 可吃 ${afterSelect.captures}`,
  )
  // h2 红炮在初始局面有 12 步，其中只有 h9 是吃子
  // （隔 h7 黑炮吃 h9 黑马 —— 黑方底线是 rnbakabnr，h9 是马、i9 才是车）
  check('落点中含 1 个可吃标记', afterSelect.captures === 1, `实际 ${afterSelect.captures}`)
  check(
    '对局页不放提示条（提示改在浮层里）',
    afterSelect.chips === 0,
    `实际 ${afterSelect.chips} 条`,
  )

  // 截图放在这里：此时正是「已选中 + 落点全部高亮」的状态，最能看出标记层对不对
  if (SHOT_PATH) {
    const shot = await client.send('Page.captureScreenshot', { format: 'png' })
    const { writeFileSync } = await import('node:fs')
    writeFileSync(SHOT_PATH, Buffer.from(shot.result.data, 'base64'))
    console.log(`\n  （截图已保存：${SHOT_PATH}）`)
  }

  console.log('\n[4] 点击 e2 应落子，界面与服务端都要更新')
  const e2 = await evaluate(client, squareExpression(4, 2))
  await clickAt(client, e2.x, e2.y)
  await sleep(500)

  const afterMove = await evaluate(
    client,
    `({
      selected: document.querySelectorAll('.piece--selected').length,
      lastMarkers: document.querySelectorAll('.fx img[src*="fx-last-move"]').length,
      moves: document.querySelectorAll('.fx img[src*="fx-move"]').length,
    })`,
  )
  check('落子后清空选中', afterMove.selected === 0, `实际 ${afterMove.selected}`)
  check(
    '上一着标记 2 个（起点 + 终点）',
    afterMove.lastMarkers === 2,
    `实际 ${afterMove.lastMarkers}`,
  )
  check('落点标记已收起', afterMove.moves === 0, `实际 ${afterMove.moves}`)

  // 界面说什么不算数，服务端说什么才算数
  const state1 = await apiState()
  check('服务端记谱为「炮二平五」', state1.history[0]?.notation === '炮二平五', `实际「${state1.history[0]?.notation}」`)
  check('服务端走子方切到黑方', state1.side === 'black', `实际 ${state1.side}`)
  check(
    '服务端 FEN 已更新',
    state1.fen.includes('b - - 1 1'),
    `实际「${state1.fen}」`,
  )

  console.log('\n[4b] 走子后应自动攒下这一手的战法讲解')
  // 讲解需要跑一次搜索（评价定级要用 root_moves），所以是异步出现的。
  // 对局页**不再显示**讲解卡（那里只放对局信息），所以这里查的是「有没有攒下来」——
  // 答案是：稍后在分析页上必须能看到它。
  await sleep(2500)

  console.log('\n[5] 悔棋应回到初始局面')
  const undoClicked = await clickButton(client, '.play__bar', '悔棋')
  check('找到并点击了悔棋按钮', undoClicked === 'ok', String(undoClicked))
  await sleep(600)

  const afterUndo = await evaluate(
    client,
    `({
      pieces: document.querySelectorAll('.piece').length,
      lastMarkers: document.querySelectorAll('.fx img[src*="fx-last-move"]').length,
    })`,
  )
  check('棋子仍为 32 枚', afterUndo.pieces === 32, `实际 ${afterUndo.pieces}`)
  check('上一着标记已清掉', afterUndo.lastMarkers === 0, `实际 ${afterUndo.lastMarkers}`)
  const state2 = await apiState()
  check('服务端记录已清空', state2.history.length === 0, `实际 ${state2.history.length} 手`)
  check('服务端走子方回到红方', state2.side === 'red', `实际 ${state2.side}`)

  console.log('\n[6] 记谱浮层应可走棋')
  const openedNotation = await clickButton(client, '.play__bar', '记谱')
  check('打开记谱浮层', openedNotation === 'ok', String(openedNotation))
  await sleep(200)

  await evaluate(
    client,
    `(() => {
      const input = document.querySelector('#notation-input');
      if (!input) return false;
      input.focus();
      return true;
    })()`,
  )
  await typeText(client, '炮二平五')
  await evaluate(
    client,
    `(() => {
      document.querySelector('.notation-form')?.requestSubmit();
      return true;
    })()`,
  )
  await sleep(600)
  const afterText = await evaluate(client, `document.querySelector('.sheet') === null`)
  check('提交后浮层自动收起', afterText === true)
  const state3 = await apiState()
  check('记谱输入走出 1 步', state3.history.length === 1, `实际 ${state3.history.length}`)
  check(
    '同样得到「炮二平五」',
    state3.history[0]?.notation === '炮二平五',
    `实际「${state3.history[0]?.notation}」`,
  )

  console.log('\n[6b] 浮层不该把棋盘挤小')
  const beforeSheet = Math.round(
    (await evaluate(client, `document.querySelector('.board').getBoundingClientRect().height`)) ?? 0,
  )
  await clickButton(client, '.play__bar', '提示')
  await sleep(300)
  const duringSheet = await evaluate(
    client,
    `({
      boardH: Math.round(document.querySelector('.board').getBoundingClientRect().height),
      hasSheet: !!document.querySelector('.sheet'),
      docScroll: document.documentElement.scrollHeight,
      winH: window.innerHeight,
    })`,
  )
  check('提示浮层已弹出', duringSheet.hasSheet === true)
  check(
    '棋盘尺寸没有变化（浮层是浮的，不占布局）',
    duringSheet.boardH === beforeSheet,
    `${beforeSheet} → ${duringSheet.boardH}`,
  )
  check('浮层没有把页面撑出滚动条', duringSheet.docScroll <= duringSheet.winH)

  let hints = 0
  await clickButton(client, '.sheet', '求引擎推荐')
  for (let i = 0; i < 80; i += 1) {
    hints = await evaluate(client, `document.querySelectorAll('.sheet .hint-chip').length`)
    if (hints >= 3) break
    await sleep(200)
  }
  check('给出 3 条推荐着法', hints === 3, `实际 ${hints} 条`)
  if (hints > 0) {
    const firstHint = await evaluate(
      client,
      `document.querySelector('.sheet .hint-chip')?.textContent ?? ''`,
    )
    console.log(`  （首条推荐：${firstHint.trim()}）`)
  }
  // 收尾前把浮层关掉，免得挡住后面的点击
  await clickButton(client, '.sheet', '关闭')
  await sleep(200)

  console.log('\n[7] 认输应结束对局并自动跳到分析页')
  // 先补一手黑棋，让这一局有 2 手 —— 只有 1 手的话复盘那一段全是退化情形：
  // 「上一手」与「回到开局」会落到同一个位置，等于什么都没测。
  await playMove(client, 7, 9, 6, 7) // 馬8进7
  const stateBeforeResign = await apiState()
  check('补的这一手走成了', stateBeforeResign.history.length === 2, `实际 ${stateBeforeResign.history.length} 手`)

  const resignOpened = await clickButton(client, '.play__bar', '认输')
  check('打开认输确认浮层', resignOpened === 'ok', String(resignOpened))
  await sleep(250)
  const confirmText = await evaluate(
    client,
    `document.querySelector('.sheet')?.textContent ?? ''`,
  )
  // 2 手之后轮到红方走，所以认输的应该是红方 —— 双人同机就是「当前该走的一方」
  check(
    '浮层说清是谁认输、且不可撤消',
    confirmText.includes('红方') && confirmText.includes('不能撤消'),
    `实际「${confirmText.slice(0, 60)}」`,
  )
  await clickButton(client, '.sheet', '确认认输')
  await sleep(1500)

  const analysis = await evaluate(
    client,
    `({
      hash: location.hash,
      verdict: document.querySelector('.verdict__headline')?.textContent ?? '',
      verdictText: document.querySelector('.verdict__text')?.textContent ?? '',
      stats: [...document.querySelectorAll('.stat')].map((s) => s.textContent.trim()),
      rows: document.querySelectorAll('.movelist__row').length,
      taught: document.querySelectorAll('.movelist__row .lv-badge').length,
      buttons: [...document.querySelectorAll('.analysis__actions .btn')].map((b) => b.textContent.trim()),
    })`,
  )
  check('自动跳到 #/analysis', analysis.hash === '#/analysis', `实际 ${analysis.hash}`)
  // 红方认输 → 赢的是黑方。认输方与胜方是**相反**的，写反了这条就永远看不出来。
  check('结果横幅显示「黑方胜」', analysis.verdict === '黑方胜', `实际「${analysis.verdict}」`)
  check(
    '结果说明指出是谁认输',
    analysis.verdictText.includes('认输'),
    `实际「${analysis.verdictText}」`,
  )
  check('统计里有总手数', analysis.stats.some((s) => s.includes('总手数')), `实际 ${analysis.stats.join(' / ')}`)
  check('逐手列表与着法数一致', analysis.rows === 2, `实际 ${analysis.rows} 行`)
  // [4b] 攒下的讲解必须在这里出现 —— 这是「走一步攒一条」的验收点
  check(
    '每一步都拿到了讲解',
    analysis.taught === 2,
    `实际 ${analysis.taught} / ${analysis.rows} 手带讲解`,
  )
  check(
    '分析页有复盘与深度分析两个入口',
    analysis.buttons.includes('复盘') && analysis.buttons.includes('深度分析'),
    `实际 ${analysis.buttons.join(' / ')}`,
  )

  console.log('\n[8] 深度分析应逐手重算并显示进度')
  await clickButton(client, '.analysis__actions', '深度分析')
  await sleep(600)
  const deepStart = await evaluate(
    client,
    `({
      label: document.querySelector('.deep__label')?.textContent ?? '',
      hasBar: !!document.querySelector('.deep__track'),
    })`,
  )
  check('出现进度条', deepStart.hasBar === true)
  check('进度条上有手数', /深度分析\s*\d+\s*\/\s*\d+/.test(deepStart.label), `实际「${deepStart.label}」`)

  let deepDone = false
  for (let i = 0; i < 150; i += 1) {
    const running = await evaluate(
      client,
      `document.querySelector('.analysis__actions .btn')?.textContent.trim() === '中止分析'`,
    )
    if (!running) {
      deepDone = true
      break
    }
    await sleep(200)
  }
  check('深度分析在 30 秒内跑完', deepDone, '仍未结束')
  const deepEnd = await evaluate(client, `document.querySelector('.deep__label')?.textContent ?? ''`)
  const [, done, total] = deepEnd.match(/(\d+)\s*\/\s*(\d+)/) ?? []
  check('覆盖了全部手数', done === total && Number(total) > 0, `实际「${deepEnd}」`)

  console.log('\n[9] 复盘应跳回对局页并能前后翻手数')
  const reviewClicked = await clickButton(client, '.analysis__actions', '复盘')
  check('点击复盘', reviewClicked === 'ok', String(reviewClicked))
  await sleep(1200)

  const atEnd = await evaluate(
    client,
    `({
      hash: location.hash,
      count: document.querySelector('.review__count')?.textContent ?? '',
      hasNote: !!document.querySelector('.review__note .note__headline'),
      clocksDimmed: document.querySelectorAll('.clock--reviewing').length,
      barButtons: [...document.querySelectorAll('.play__bar .btn')].map((b) => b.textContent.trim()),
      barBottom: Math.round(document.querySelector('.play__bar').getBoundingClientRect().bottom),
      docScroll: document.documentElement.scrollHeight,
      winH: window.innerHeight,
    })`,
  )
  check('回到 #/play', atEnd.hash === '#/play', `实际 ${atEnd.hash}`)
  check('复盘条显示「第 2 / 2 手」', /第\s*2\s*\/\s*2\s*手/.test(atEnd.count), `实际「${atEnd.count}」`)
  check('复盘条里有这一手的讲解', atEnd.hasNote)
  check('两条钟都进入复盘态（不再显示读秒）', atEnd.clocksDimmed >= 1, `实际 ${atEnd.clocksDimmed}`)
  check(
    '复盘时按钮换成「返回分析」',
    atEnd.barButtons.includes('返回分析') && !atEnd.barButtons.includes('认输'),
    `实际 ${atEnd.barButtons.join(' / ')}`,
  )
  check('复盘条在视口之内', atEnd.barBottom <= atEnd.winH, `${atEnd.barBottom} / ${atEnd.winH}`)
  check('复盘页也没有滚动条', atEnd.docScroll <= atEnd.winH)

  // 上一手
  await evaluate(
    client,
    `(() => {
      const btn = document.querySelector('[aria-label="上一手"]');
      if (!btn || btn.disabled) return false;
      btn.click();
      return true;
    })()`,
  )
  await sleep(900)
  const prev = await evaluate(
    client,
    `({
      count: document.querySelector('.review__count')?.textContent ?? '',
      rows: document.querySelectorAll('.piece').length,
    })`,
  )
  check('翻到「第 1 / 2 手」', /第\s*1\s*\/\s*2\s*手/.test(prev.count), `实际「${prev.count}」`)
  check('棋盘仍是 32 枚棋子（无吃子局面）', prev.rows === 32, `实际 ${prev.rows}`)

  // 回开局：此时「上一手」应变成不可用
  await evaluate(
    client,
    `(() => {
      const btn = document.querySelector('[aria-label="回到开局"]');
      if (!btn || btn.disabled) return false;
      btn.click();
      return true;
    })()`,
  )
  await sleep(900)
  const start = await evaluate(
    client,
    `({
      count: document.querySelector('.review__count')?.textContent ?? '',
      prevDisabled: document.querySelector('[aria-label="上一手"]')?.disabled ?? null,
      note: document.querySelector('.review__note')?.textContent ?? '',
    })`,
  )
  check('翻到「第 0 / 2 手」', /第\s*0\s*\/\s*2\s*手/.test(start.count), `实际「${start.count}」`)
  check('开局处「上一手」已禁用', start.prevDisabled === true, `disabled=${start.prevDisabled}`)
  check('开局处给出解释而不是空白', start.note.includes('开局'), `实际「${start.note.slice(0, 40)}」`)
  const stateReview = await apiState()
  check('服务端游标也跟着回到 0', stateReview.cursor === 0, `实际 ${stateReview.cursor}`)
  check('服务端记录没有被复盘截断', stateReview.history.length === 2, `实际 ${stateReview.history.length}`)

  console.log('\n[10] 人机对战（我执黑）：引擎应自动走一步')
  await evaluate(client, `location.hash = '#/setup'`)
  await sleep(600)
  await evaluate(
    client,
    `(() => {
      const group = document.querySelector('[aria-label="对局模式"]');
      const btn = [...group.querySelectorAll('button')].find((b) => b.textContent.trim() === '人机对战');
      btn.click();
      return true;
    })()`,
  )
  await sleep(250)
  const hasSideCard = await evaluate(
    client,
    `!!document.querySelector('[aria-label="我执哪一方"]')`,
  )
  check('切到人机对战后出现「我执哪一方」', hasSideCard === true)

  await evaluate(
    client,
    `(() => {
      const group = document.querySelector('[aria-label="我执哪一方"]');
      const btn = [...group.querySelectorAll('button')].find((b) => b.textContent.trim() === '黑方');
      btn.click();
      return true;
    })()`,
  )
  await sleep(200)
  const started2 = await startGame(client)
  check('开局进入对局页', started2 === 'ok', String(started2))

  // 等引擎落子（默认中级档，思考约 1.2 秒；给足余量）
  let engineMoved = false
  for (let i = 0; i < 100; i += 1) {
    const n = await apiState()
    if (n.history.length >= 1) {
      engineMoved = true
      break
    }
    await sleep(200)
  }
  check('引擎自动走了一步', engineMoved, '等待 20 秒后服务端记录仍为空')

  const engineView = await evaluate(
    client,
    `({
      thinking: !!document.querySelector('.app__thinking'),
      interactive: !document.querySelector('.board--locked'),
      lastMarkers: document.querySelectorAll('.fx img[src*="fx-last-move"]').length,
    })`,
  )
  check('引擎落子后思考指示已消失', !engineView.thinking)
  check('棋盘上标出了引擎走的那一步', engineView.lastMarkers === 2, `实际 ${engineView.lastMarkers}`)
  check('轮到玩家走时棋盘恢复可点', engineView.interactive)

  // 收尾截图：此时处于人机对战 + 一屏对局页的状态（下一段会把局面走掉）
  if (SHOT_PATH_END) {
    const shot = await client.send('Page.captureScreenshot', { format: 'png' })
    const { writeFileSync } = await import('node:fs')
    writeFileSync(SHOT_PATH_END, Buffer.from(shot.result.data, 'base64'))
    console.log(`\n（收尾截图已保存：${SHOT_PATH_END}）`)
  }

  console.log('\n[11] 步时归零应**当场**判超时，并自动跳到分析页')
  // 这一条盯的是一个真实报过的缺陷：钟走到 0 之后**什么都不会发生**，
  // 得再点一下棋盘才被判负 —— 而那一刻玩家很可能已经走开了。
  // 它是唯一会让这个脚本慢 20 多秒的一段，但值得：这是全项目唯一
  // 端到端覆盖「超时自动生效」的地方。
  await evaluate(client, `location.hash = '#/setup'`)
  await waitFor(client, '.setup__start')
  await clickButton(client, '[aria-label="对局模式"]', '双人同机')
  await clickButton(client, '[aria-label="限时档位"]', '快棋')
  await sleep(250)
  check('换回双人同机并开了快棋', (await startGame(client)) === 'ok')

  const startedAt = Date.now()
  // 一步都不走，看它会不会自己判
  let autoEnded = null
  for (let i = 0; i < 160; i += 1) {
    if ((await evaluate(client, `location.hash`)) === '#/analysis') {
      autoEnded = Date.now() - startedAt
      break
    }
    await sleep(200)
  }
  check('32 秒内自动判定并跳到分析页', autoEnded !== null, '步时归零后仍未跳转')
  if (autoEnded !== null) {
    console.log(`  （从开局到自动判负：${(autoEnded / 1000).toFixed(1)} 秒 · 步时 20 秒）`)
    const text = await evaluate(
      client,
      `document.querySelector('.verdict__text')?.textContent ?? ''`,
    )
    check('终局原因是超时判负', text.includes('超时'), `实际「${text}」`)
  }

  console.log('\n[12] 机机对战：两个 AI 自己走，能暂停、能单步')
  await evaluate(client, `location.hash = '#/setup'`)
  await waitFor(client, '.setup__start')
  check('模式里有「机机对战」', (await clickButton(client, '[aria-label="对局模式"]', '机机对战')) === 'ok')
  await sleep(250)

  const levelConfig = await evaluate(
    client,
    `({
      red: !!document.querySelector('[aria-label="红方棋力"]'),
      black: !!document.querySelector('[aria-label="黑方棋力"]'),
      noSide: !document.querySelector('[aria-label="我执哪一方"]'),
    })`,
  )
  check('两边棋力可以分别选', levelConfig.red && levelConfig.black)
  check('不再问「我执哪一方」（那是人机对战才有的）', levelConfig.noSide)

  // 挑两个快档位，这一段的等待才不至于太长
  await clickButton(client, '[aria-label="红方棋力"]', '入门')
  await clickButton(client, '[aria-label="黑方棋力"]', '初级')
  await sleep(200)
  check('开局', (await startGame(client)) === 'ok')

  const autoView = await evaluate(
    client,
    `({
      buttons: [...document.querySelectorAll('.play__bar .btn')].map((b) => b.textContent.trim()),
      levels: [...document.querySelectorAll('.clock__level')].map((e) => e.textContent.trim()),
      locked: !!document.querySelector('.board--locked'),
    })`,
  )
  check(
    '按钮换成 暂停 / 单步 / 翻转 / 返回设置',
    autoView.buttons.join('/') === '暂停/单步/翻转/返回设置',
    `实际 ${autoView.buttons.join('/')}`,
  )
  check(
    '两条钟各自标出了是哪一档 AI',
    autoView.levels.length === 2,
    `实际 ${autoView.levels.join(' / ') || '(无)'}`,
  )
  check('机机对战里棋盘是只读的（人是观众）', autoView.locked === true)

  let autoMoves = 0
  for (let i = 0; i < 60; i += 1) {
    autoMoves = (await apiState()).history.length
    if (autoMoves >= 3) break
    await sleep(300)
  }
  check('两个 AI 自己走出了至少 3 手', autoMoves >= 3, `实际 ${autoMoves} 手`)

  await clickButton(client, '.play__bar', '暂停')
  // ⚠️ 暂停**不会打断已经在飞的那一手** —— 引擎已经算起来了，拦不住，
  // 它落地之后才真正停下来。所以基准要在它落地之后再取，否则会把
  // 「这一手正常落子」误判成「暂停没生效」。这个坑第一版就踩了。
  let settled = null
  for (let i = 0; i < 40; i += 1) {
    const a = (await apiState()).history.length
    await sleep(600)
    const b = (await apiState()).history.length
    if (a === b) {
      settled = a
      break
    }
  }
  check('暂停后局面稳定下来', settled !== null)

  const pausedText = await evaluate(
    client,
    `document.querySelector('.play__mode')?.textContent ?? ''`,
  )
  check('状态变成「已暂停」', pausedText.includes('已暂停'), `实际「${pausedText}」`)

  const beforePause = settled ?? (await apiState()).history.length
  await sleep(4_000)
  const afterPause = (await apiState()).history.length
  check('暂停期间两个 AI 都停手', afterPause === beforePause, `${beforePause} → ${afterPause}`)

  // 单步：等按钮解禁（暂停那一刻可能还有一手在飞）再点
  let stepClicked = 'disabled'
  for (let i = 0; i < 40 && stepClicked === 'disabled'; i += 1) {
    stepClicked = await clickButton(client, '.play__bar', '单步')
    if (stepClicked === 'disabled') await sleep(200)
  }
  check('单步按钮解禁并可点', stepClicked === 'ok', String(stepClicked))

  let stepped = beforePause
  for (let i = 0; i < 40; i += 1) {
    stepped = (await apiState()).history.length
    if (stepped > beforePause) break
    await sleep(200)
  }
  check('单步恰好走了一手', stepped === beforePause + 1, `${beforePause} → ${stepped}`)
  await sleep(2_500)
  check(
    '单步之后仍然停着（没有溜回连播）',
    (await apiState()).history.length === stepped,
    `实际 ${(await apiState()).history.length} 手`,
  )

  console.log('\n[13] 机机对战：档位默认随机、选了就固定、再点「随机」交还随机')
  // 刷新一下再验「默认」—— 前面的段落已经把两边的档位选死了，
  // 不重置的话看到的不是出厂默认，而是上一段留下的残局（第一版就踩了这个）。
  await client.send('Page.reload', {})
  await waitFor(client, '.setup__start')
  await clickButton(client, '[aria-label="对局模式"]', '机机对战')
  await sleep(400)

  const rolled = await evaluate(
    client,
    `({
      red: document.querySelector('[aria-label="红方棋力"] button[aria-pressed="true"]')?.textContent.trim(),
      black: document.querySelector('[aria-label="黑方棋力"] button[aria-pressed="true"]')?.textContent.trim(),
      hint: [...document.querySelectorAll('.setup__hint')].map((p) => p.textContent.trim()).find((t) => t.startsWith('本局')),
    })`,
  )
  check('两边默认都是「随机」', rolled.red === '随机' && rolled.black === '随机', `${rolled.red} / ${rolled.black}`)
  check(
    '页面上说清了本局实际用哪两档',
    /红方\s*\S+（随机）/.test(rolled.hint ?? ''),
    `实际「${rolled.hint}」`,
  )

  await clickButton(client, '[aria-label="红方棋力"]', '大师')
  await sleep(200)
  const picked = await evaluate(
    client,
    `({
      red: document.querySelector('[aria-label="红方棋力"] button[aria-pressed="true"]')?.textContent.trim(),
      black: document.querySelector('[aria-label="黑方棋力"] button[aria-pressed="true"]')?.textContent.trim(),
    })`,
  )
  check('手动选过的一方不再是「随机」', picked.red === '大师', `实际「${picked.red}」`)
  check('没动过的那一方还是「随机」', picked.black === '随机', `实际「${picked.black}」`)

  await clickButton(client, '[aria-label="红方棋力"]', '随机')
  await sleep(200)
  const backToRandom = await evaluate(
    client,
    `document.querySelector('[aria-label="红方棋力"] button[aria-pressed="true"]')?.textContent.trim()`,
  )
  check('再点「随机」就把这一方交还给随机', backToRandom === '随机', `实际「${backToRandom}」`)

  console.log('\n[14] 机机对战：两次落子之间至少隔 3 秒')
  // ⚠️ 这条测的是**节奏**，不是「让引擎多想」。实测入门档只想 2 层、
  // 给它 3000 毫秒预算它 0 毫秒就返回了 —— 弱档位对时间预算免疫，
  // 所以「每步至少 3 秒」只能靠走子节奏的下限来保证。
  await clickButton(client, '[aria-label="红方棋力"]', '入门')
  await clickButton(client, '[aria-label="黑方棋力"]', '入门')
  await sleep(200)
  check('开局', (await startGame(client)) === 'ok')

  const stamps = []
  let seen = null
  for (let i = 0; i < 320 && stamps.length < 4; i += 1) {
    const n = (await apiState()).history.length
    if (seen !== null && n !== seen) stamps.push(Date.now())
    seen = n
    await sleep(60)
  }
  check('观察到至少 3 次落子', stamps.length >= 3, `实际 ${stamps.length} 次`)
  if (stamps.length >= 3) {
    const gaps = stamps.slice(1).map((t, i) => (t - stamps[i]) / 1000)
    const min = Math.min(...gaps)
    console.log(`  （相邻两手间隔：${gaps.map((g) => g.toFixed(2)).join(' / ')} 秒）`)
    // 采样本身有 60 毫秒粒度，测出来的间隔只会偏短，所以门槛留一点余量
    check('最小间隔不低于 3 秒', min >= 2.9, `最小 ${min.toFixed(2)} 秒`)
  }

  console.log('\n[15] 机机对战：一局完了不跳分析页，倒计时后自动开下一局')
  // 这是整个脚本里最慢的一段（要等一局真下完，一两分钟），
  // 但它是「连续对局」这个功能的唯一端到端覆盖 —— 没有别的办法能验它。
  await clickButton(client, '.play__bar', '暂停')
  await sleep(300)
  await evaluate(client, `location.hash = '#/setup'`)
  await waitFor(client, '.setup__start')
  await clickButton(client, '[aria-label="红方棋力"]', '入门')
  await clickButton(client, '[aria-label="黑方棋力"]', '初级')
  await sleep(200)
  check('开局（入门 vs 初级，这一局结束得快）', (await startGame(client)) === 'ok')

  const recordBefore = await evaluate(
    client,
    `document.querySelector('.clock__record')?.textContent ?? ''`,
  )
  check('钟条上先记着 0 胜', recordBefore.includes('0 胜'), `实际「${recordBefore}」`)

  let endMoves = 0
  let finished = false
  // 预算给到 5 分钟。这不是「慢」而是**没法预测**：AI 对局多长取决于两边怎么走，
  // 实测同一对档位下既有 10 手就分出胜负的，也有走了几十手还在缠斗的。
  // 六回合自然限着能保证它一定会结束，但那个上限是 120 手（约 6 分钟），
  // 卡太紧会变成一个时灵时不灵的用例 —— 那种用例比没有还糟。
  for (let i = 0; i < 600; i += 1) {
    const s = await apiState()
    if (s.status.over) {
      endMoves = s.history.length
      finished = true
      break
    }
    await sleep(500)
  }
  check('五分钟内下完一局', finished, '还没下完，后面几条验不了')

  if (finished) {
    // 让「终局 → 记战绩 → 排下一局」这几个 effect 跑完再读界面，
    // 否则可能读到还没更新的那一帧，把一个正常的时序误判成缺陷
    await sleep(800)
    const gapView = await evaluate(
      client,
      `({
        hash: location.hash,
        countdown: document.querySelector('.play__mode')?.textContent ?? '',
        record: document.querySelector('.clock__record')?.textContent ?? '',
        buttons: [...document.querySelectorAll('.play__bar .btn')].map((b) => b.textContent.trim()),
      })`,
    )
    check('留在对局页，没有跳去分析页', gapView.hash === '#/play', `实际 ${gapView.hash}`)
    check(
      '倒计时说清了多久后开下一局',
      /秒后开始下一局/.test(gapView.countdown),
      `实际「${gapView.countdown}」`,
    )
    check('战绩加了一笔', /[1-9]\d* 胜/.test(gapView.record), `实际「${gapView.record}」`)
    check(
      '终局后「单步」换成「看分析」',
      gapView.buttons.includes('看分析') && !gapView.buttons.includes('单步'),
      `实际 ${gapView.buttons.join('/')}`,
    )

    // 等它自己开下一局
    let restarted = false
    for (let i = 0; i < 90; i += 1) {
      const s = await apiState()
      if (!s.status.over && s.history.length < endMoves) {
        restarted = true
        break
      }
      await sleep(500)
    }
    check('倒计时结束后自动开了下一局', restarted, `局面一直停在 ${endMoves} 手`)

    // 停下来，别把连播留给收尾
    await clickButton(client, '.play__bar', '暂停')
    await sleep(300)
  }

  console.log('\n[16] 钟条上显示自然限着的进度')
  // 用「双人同机 + 不限时」来测：这样钟条左端**只有**限着这一项，
  // 断言不用从一串「局时 · 步时 · 读秒」里挑，也不受时间档位影响。
  await evaluate(client, `location.hash = '#/setup'`)
  await waitFor(client, '.setup__start')
  await clickButton(client, '[aria-label="对局模式"]', '双人同机')
  await clickButton(client, '[aria-label="限时档位"]', '不限时')
  await sleep(250)
  check('开局（双人同机 · 不限时）', (await startGame(client)) === 'ok')

  const limitText = () =>
    evaluate(client, `document.querySelector('.clock__config')?.textContent.trim() ?? ''`)

  const atStart = await limitText()
  check('开局时是 0/60 回合', atStart.includes('限着 0/60 回合'), `实际「${atStart}」`)

  // 走两个半步（都不吃子）→ 满一个回合
  await playMove(client, 7, 2, 4, 2) // 炮二平五
  await playMove(client, 7, 9, 6, 7) // 馬8进7
  const afterTwo = await limitText()
  check('两个半步之后记到 1/60 回合', afterTwo.includes('限着 1/60 回合'), `实际「${afterTwo}」`)

  // 吃子要把计数清零 —— 这是这个计数存在的意义
  await playMove(client, 4, 2, 4, 6) // 炮五进四，吃掉黑方中卒
  const afterCapture = await limitText()
  check('吃子之后计数清零', afterCapture.includes('限着 0/60 回合'), `实际「${afterCapture}」`)

  console.log('\n[17] 机机对战：随时叫停，回到开局前')
  // 「暂停」只是把 20 秒倒计时按住，继续一按又是一局。看够了几十局想收手，
  // 得有出口 —— 这一节验的是那个出口真把连播**停住了**（不只是跳走了页面）。
  await evaluate(client, `location.hash = '#/setup'`)
  await waitFor(client, '.setup__start')
  await clickButton(client, '[aria-label="对局模式"]', '机机对战')
  await sleep(250)
  check('开局（机机对战）', (await startGame(client)) === 'ok')

  const pliesAt = async () => (await apiState()).history.length
  await sleep(4000)
  const pliesBefore = await pliesAt()
  check('连播在走棋', pliesBefore > 0, `手数 ${pliesBefore}`)

  const clicked = await clickButton(client, '.play__bar', '返回设置')
  check('按钮行里有「返回设置」', clicked === 'ok', `clickButton 返回 ${clicked}`)
  await sleep(2500)
  const backHash = await evaluate(client, 'location.hash')
  check('点「返回设置」回到设置页', backHash === '#/setup', `实际 ${backHash}`)

  // 先等一手：点下去那一刻可能正好有一步棋在飞，等它落定再取基准，
  // 否则「叫停后没再走」会被那一步误判成失败
  const pliesAtStop = await pliesAt()
  await sleep(6000)
  const pliesLater = await pliesAt()
  check(
    '叫停之后棋不再往下走',
    pliesLater === pliesAtStop,
    `叫停时 ${pliesAtStop} 手，六秒后 ${pliesLater} 手`,
  )

  // 关掉自己建的标签页，不碰别的
  await fetch(`${CDP}/json/close/${client.targetId}`).catch(() => {})
  client.close()

  // 复位，别把走了一半的局面留给下一次运行或用户
  await resetGame()

  console.log('\n────────────────────────────────')
  if (failures === 0) {
    console.log('ui_probe: 全部通过 ✓')
  } else {
    console.log(`ui_probe: ${failures} 项失败 ✗`)
    process.exit(1)
  }
}

main().catch((error) => {
  console.error('ui_probe 运行失败：', error)
  process.exit(1)
})
