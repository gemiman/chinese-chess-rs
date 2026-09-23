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
 */

const CDP_PORT = Number(process.env.CDP_PORT ?? 9222)
const PAGE_URL = process.argv[2] ?? 'http://127.0.0.1:8848/'
const SHOT_PATH = process.env.SHOT_PATH ?? null
const SHOT_PATH_END = process.env.SHOT_PATH_END ?? null

const CDP = `http://127.0.0.1:${CDP_PORT}`
const ORIGIN = new URL(PAGE_URL).origin

/**
 * 视口必须比棋盘大。
 *
 * 无头 Chrome 的默认视口只有 762×484，而棋盘是 560×620 —— 底部的棋子
 * （`row` 0..2 那几排）会落在视口外。此时点击事件的坐标虽然在页面坐标系里
 * 算得对，浏览器却根本不会把它派发到那个位置，表现为「点了没反应」。
 * 这个坑踩过一次，所以这里显式设定。
 */
const VIEWPORT = { width: 1280, height: 1100 }

// 与 frontend/src/coords.ts 保持一致
const MARGIN = 40
const CELL = 60
const VIEW_W = 560
const VIEW_H = 620
const ROWS = 10

let failures = 0

function check(label, ok, detail = '') {
  if (ok) {
    console.log(`  \u2713 ${label}`)
  } else {
    failures += 1
    console.log(`  \u2717 ${label}${detail ? ` —— ${detail}` : ''}`)
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

  // 等应用挂载并完成首次 /api/state 拉取
  for (let i = 0; i < 60; i += 1) {
    const ready = await evaluate(client, `!!document.querySelector('.board')`)
    if (ready) break
    await sleep(100)
  }
  await sleep(200)

  console.log('\n[1] 初始渲染')
  const board = await evaluate(
    client,
    `(() => {
      const b = document.querySelector('.board');
      if (!b) return null;
      const r = b.getBoundingClientRect();
      return {
        width: Math.round(r.width),
        height: Math.round(r.height),
        pieces: document.querySelectorAll('.piece').length,
        boardImg: !!document.querySelector('.board__surface'),
      };
    })()`,
  )
  check('棋盘已渲染', board !== null)
  if (!board) {
    client.close()
    process.exit(1)
  }
  check('棋子 32 枚', board.pieces === 32, `实际 ${board.pieces}`)
  check('棋盘底图已加载', board.boardImg)

  const expectedRatio = VIEW_H / VIEW_W
  const actualRatio = board.height / board.width
  check(
    `棋盘宽高比 ${expectedRatio.toFixed(3)}`,
    Math.abs(actualRatio - expectedRatio) < 0.01,
    `实际 ${actualRatio.toFixed(3)}（宽高比不对会导致棋子与格线错位）`,
  )

  console.log('\n[2] 点击红炮 h2 应选中并高亮 12 个落点')
  const h2 = await evaluate(client, squareExpression(7, 2))
  await clickAt(client, h2.x, h2.y)
  await sleep(250)

  const afterSelect = await evaluate(
    client,
    `({
      selected: document.querySelectorAll('.piece--selected').length,
      dots: document.querySelectorAll('.marker--dot').length,
      rings: document.querySelectorAll('.marker--ring').length,
      chips: document.querySelectorAll('.hint-chip').length,
      banner: document.querySelector('.status__text')?.textContent ?? '',
    })`,
  )
  check('恰好一枚棋子被选中', afterSelect.selected === 1, `实际 ${afterSelect.selected}`)
  check('侧栏出现走棋提示', afterSelect.chips === 12, `期望 12 条，实际 ${afterSelect.chips}`)
  check(
    '棋盘上有落点标记（点 + 环）',
    afterSelect.dots + afterSelect.rings === 12,
    `点 ${afterSelect.dots} + 环 ${afterSelect.rings}`,
  )

  // h2 红炮在初始局面有 12 步，其中只有 h9 是吃子
  // （隔 h7 黑炮吃 h9 黑马 —— 黑方底线是 rnbakabnr，h9 是马、i9 才是车）
  check('落点中含 1 个可吃标记', afterSelect.rings === 1, `实际 ${afterSelect.rings}`)

  // 截图放在这里：此时正是「已选中 + 落点全部高亮」的状态，最能看出标记层对不对
  if (SHOT_PATH) {
    const shot = await client.send('Page.captureScreenshot', { format: 'png' })
    const { writeFileSync } = await import('node:fs')
    writeFileSync(SHOT_PATH, Buffer.from(shot.result.data, 'base64'))
    console.log(`\n  （截图已保存：${SHOT_PATH}）`)
  }

  console.log('\n[3] 点击 e2 应落子并更新记谱')
  const e2 = await evaluate(client, squareExpression(4, 2))
  await clickAt(client, e2.x, e2.y)
  await sleep(400)

  const afterMove = await evaluate(
    client,
    `({
      rows: document.querySelectorAll('.moves__row').length,
      firstRed: document.querySelector('.moves__red')?.textContent ?? '',
      turn: document.querySelector('.status__meta')?.textContent ?? '',
      selected: document.querySelectorAll('.piece--selected').length,
      lastMarkers: document.querySelectorAll('.board__layer--overlay .marker--last').length,
      fen: document.querySelector('.fen')?.textContent ?? '',
    })`,
  )
  check('记谱出现 1 行', afterMove.rows === 1, `实际 ${afterMove.rows}`)
  check('记谱内容为「炮二平五」', afterMove.firstRed === '炮二平五', `实际「${afterMove.firstRed}」`)
  check('落子后清空选中', afterMove.selected === 0, `实际 ${afterMove.selected}`)
  check(
    '上一着标记 2 个（且在棋子之上的 overlay 层）',
    afterMove.lastMarkers === 2,
    `实际 ${afterMove.lastMarkers}`,
  )
  check(
    '走子方已切换为黑方',
    afterMove.turn.includes('轮到 黑方'),
    `实际「${afterMove.turn.trim()}」`,
  )
  check(
    'FEN 已更新',
    afterMove.fen.includes('b - - 1 1'),
    `实际「${afterMove.fen}」`,
  )

  console.log('\n[3b] 走子后应自动出现战法讲解')
  // 讲解需要跑一次搜索（评价定级要用 root_moves），所以是异步出现的
  let coachReady = false
  for (let i = 0; i < 100; i += 1) {
    if (await evaluate(client, `!!document.querySelector('.note__headline')`)) {
      coachReady = true
      break
    }
    await sleep(200)
  }
  check('讲解卡片已出现', coachReady, '等待 20 秒后仍未出现')

  if (coachReady) {
    const coach = await evaluate(
      client,
      `(() => {
        const card = document.querySelector('.note');
        const text = card ? card.textContent : '';
        return {
          badge: document.querySelector('.lv-badge')?.textContent?.trim() ?? '',
          headline: document.querySelector('.note__headline')?.textContent ?? '',
          tactics: [...document.querySelectorAll('.tactic-chip')].map((e) => e.textContent.trim()).join('|'),
          hasPlaceholder: /[{}]/.test(text),
          fallback: text.includes('兜底模板'),
        };
      })()`,
    )
    // 等级徽标必须同时有图标形状与文字 —— 不允许仅靠颜色传达等级
    check(
      '等级徽标含图标与文字',
      /[★✓?✕‼]/.test(coach.badge) && coach.badge.length > 2,
      `实际「${coach.badge}」`,
    )
    check('讲解正文非空', coach.headline.length > 4, `实际「${coach.headline}」`)
    check('讲解里没有残留占位符', !coach.hasPlaceholder, coach.headline)
    check('首步讲解未落到兜底模板', !coach.fallback, '开局着法不应走兜底')
    check('首步识别出开局类战术', coach.tactics.includes('中炮'), `实际「${coach.tactics}」`)
    console.log(`  （首步讲解：${coach.headline} · 战术：${coach.tactics}）`)
  }

  console.log('\n[4] 悔棋应回到初始局面')
  const undoClicked = await evaluate(
    client,
    `(() => {
      const btn = [...document.querySelectorAll('.btn')].find((b) => b.textContent.trim() === '悔棋');
      if (!btn) return false;
      btn.click();
      return true;
    })()`,
  )
  check('找到并点击了悔棋按钮', undoClicked === true)
  await sleep(400)

  const afterUndo = await evaluate(
    client,
    `({
      rows: document.querySelectorAll('.moves__row').length,
      pieces: document.querySelectorAll('.piece').length,
      turn: document.querySelector('.status__meta')?.textContent ?? '',
    })`,
  )
  check('记谱清空', afterUndo.rows === 0, `实际 ${afterUndo.rows}`)
  check('棋子仍为 32 枚', afterUndo.pieces === 32, `实际 ${afterUndo.pieces}`)
  check('走子方回到红方', afterUndo.turn.includes('轮到 红方'), `实际「${afterUndo.turn.trim()}」`)

  console.log('\n[5] 记谱输入框应可走棋')
  await evaluate(
    client,
    `(() => {
      const input = document.querySelector('#notation-input');
      input.focus();
      return true;
    })()`,
  )
  await typeText(client, '炮二平五')
  await evaluate(
    client,
    `(() => {
      document.querySelector('.notation-form').requestSubmit();
      return true;
    })()`,
  )
  await sleep(400)
  const afterText = await evaluate(
    client,
    `({
      rows: document.querySelectorAll('.moves__row').length,
      firstRed: document.querySelector('.moves__red')?.textContent ?? '',
    })`,
  )
  check('记谱输入走出 1 步', afterText.rows === 1, `实际 ${afterText.rows}`)
  check('同样得到「炮二平五」', afterText.firstRed === '炮二平五', `实际「${afterText.firstRed}」`)

  console.log('\n[6] 切到人机对战（我执黑），引擎应自动走一步')
  await resetGame()
  // 重新加载页面，确保从干净的初始局面开始
  await client.send('Page.navigate', { url: PAGE_URL })
  await sleep(600)
  for (let i = 0; i < 60; i += 1) {
    if (await evaluate(client, `!!document.querySelector('.board')`)) break
    await sleep(100)
  }

  const clickedMode = await evaluate(
    client,
    `(() => {
      const group = document.querySelector('[aria-label="对局模式"]');
      if (!group) return 'no-group';
      const btn = [...group.querySelectorAll('button')].find((b) => b.textContent.trim() === '人机对战');
      if (!btn) return 'no-button';
      btn.click();
      return 'ok';
    })()`,
  )
  check('找到并点击「人机对战」', clickedMode === 'ok', String(clickedMode))

  const clickedSide = await evaluate(
    client,
    `(() => {
      const group = document.querySelector('[aria-label="我执哪一方"]');
      if (!group) return 'no-group';
      const btn = [...group.querySelectorAll('button')].find((b) => b.textContent.trim() === '黑方');
      if (!btn) return 'no-button';
      btn.click();
      return 'ok';
    })()`,
  )
  check('找到并点击「我执黑方」', clickedSide === 'ok', String(clickedSide))

  // 等引擎落子（默认中级档，思考约 1.2 秒；给足余量）
  let engineMoved = false
  for (let i = 0; i < 100; i += 1) {
    const rows = await evaluate(client, `document.querySelectorAll('.moves__row').length`)
    if (rows >= 1) {
      engineMoved = true
      break
    }
    await sleep(200)
  }
  check('引擎自动走了一步', engineMoved, '等待 20 秒后记谱仍为空')

  if (engineMoved) {
    const engineView = await evaluate(
      client,
      `({
        firstRed: document.querySelector('.moves__red')?.textContent ?? '',
        info: document.querySelector('.engine-line')?.textContent ?? '',
        thinking: !!document.querySelector('.app__thinking'),
        interactive: !document.querySelector('.board--locked'),
      })`,
    )
    check('记谱里有红方第一着', engineView.firstRed.length > 0, `实际「${engineView.firstRed}」`)
    check(
      '显示了引擎搜索信息',
      /深度\s*\d+/.test(engineView.info),
      `实际「${engineView.info.trim()}」`,
    )
    check('引擎落子后思考指示已消失', !engineView.thinking)
    check('轮到玩家走时棋盘恢复可点', engineView.interactive)
  }

  console.log('\n[7] 求引擎推荐应给出 3 条候选')
  const hintClicked = await evaluate(
    client,
    `(() => {
      const btn = [...document.querySelectorAll('.btn')].find((b) => b.textContent.trim() === '求引擎推荐');
      if (!btn) return 'not-found';
      if (btn.disabled) return 'disabled';
      btn.click();
      return 'ok';
    })()`,
  )
  check('找到并点击「求引擎推荐」', hintClicked === 'ok', String(hintClicked))

  let hints = 0
  for (let i = 0; i < 80; i += 1) {
    hints = await evaluate(client, `document.querySelectorAll('.hint-chip').length`)
    if (hints >= 3) break
    await sleep(200)
  }
  check('给出 3 条推荐着法', hints === 3, `实际 ${hints} 条`)

  // 记录一条推荐着法的文案，便于人工核对
  if (hints > 0) {
    const firstHint = await evaluate(
      client,
      `document.querySelector('.hint-chip')?.textContent ?? ''`,
    )
    console.log(`  （首条推荐：${firstHint.trim()}）`)
  }

  // 收尾截图：此时处于人机对战 + 已显示推荐着法的状态，最能看出面板布局有没有问题
  if (SHOT_PATH_END) {
    const shot = await client.send('Page.captureScreenshot', { format: 'png' })
    const { writeFileSync } = await import('node:fs')
    writeFileSync(SHOT_PATH_END, Buffer.from(shot.result.data, 'base64'))
    console.log(`\n（收尾截图已保存：${SHOT_PATH_END}）`)
  }

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
