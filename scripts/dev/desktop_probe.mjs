/**
 * 桌面端（Tauri）冒烟测试 —— 连 WebView2 的调试端口，验证真机 shell。
 *
 * # 为什么需要它
 *
 * `ui_probe.mjs` 走的是 HTTP 桥，能覆盖「棋盘渲染 + 主交互 + 引擎应招」，
 * 但它**碰不到 Tauri shell 这一层**：命令有没有注册、参数名 camelCase 映射对不对、
 * `async fn` 有没有真的丢到线程池上，这些都只有在真正的 WebView2 里调一次才知道。
 *
 * 更现实的理由：本项目真的踩过「桌面端走过一步后让引擎应招，桥接层线程 panic，
 * 前端只看到响应中途断掉」这个坑，而当时没有任何自动化测试能发现它。
 *
 * # 用法
 *
 * ```bash
 * # 1. 带调试端口启动桌面端（PowerShell）
 * $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9223"
 * cargo run -p xq-client
 *
 * # 2. 跑本脚本
 * node scripts/dev/desktop_probe.mjs
 * ```
 *
 * 退出码 0 = 全部通过；1 = 有用例失败。
 */

const CDP_PORT = Number(process.env.DESKTOP_CDP_PORT ?? 9223)
const CDP = `http://127.0.0.1:${CDP_PORT}`

let failures = 0

function check(label, ok, detail = '') {
  if (ok) {
    console.log(`  \u2713 ${label}`)
  } else {
    failures += 1
    console.log(`  \u2717 ${label}${detail ? ` \u2014\u2014 ${detail}` : ''}`)
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

/** 连上 WebView2 里的页面目标。 */
async function connectPage() {
  let list
  try {
    list = await (await fetch(`${CDP}/json/list`)).json()
  } catch (cause) {
    throw new Error(
      `连不上 ${CDP}。桌面端是否带 --remote-debugging-port=${CDP_PORT} 启动过？\n` +
        `（原始错误：${cause instanceof Error ? cause.message : String(cause)}）`,
    )
  }
  const page = list.find((t) => t.type === 'page')
  if (!page) throw new Error('WebView2 里没有找到 page 目标')

  const socket = new WebSocket(page.webSocketDebuggerUrl)
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

  return { url: page.url, send, close: () => socket.close() }
}

/** 在页面里求值，返回 JSON 化后的结果。 */
async function evaluate(client, expression) {
  const reply = await client.send('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true,
  })
  const details = reply.result?.exceptionDetails
  if (details) {
    throw new Error(`页面内求值抛错：${details.exception?.description ?? details.text}`)
  }
  return reply.result?.result?.value
}

/**
 * 直接调一个 Tauri command 并返回 { ok, value | error }。
 *
 * 刻意不用页面的 `bridge` 对象：这一层测的就是「命令注册 + 参数映射」本身，
 * 绕过前端封装才能区分「命令坏了」和「前端封装坏了」。
 */
function invokeExpression(cmd, args) {
  return `(async () => {
    try {
      const value = await window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args ?? {})});
      return { ok: true, value };
    } catch (e) {
      return { ok: false, error: String(e) };
    }
  })()`
}

/**
 * 按文字点击一个按钮，返回是否找到并点中。
 *
 * `scopeSelector` 是一个普通 CSS 选择器（默认整页）。刻意不用 `aria-label` 专用参数：
 * 按钮的归属容器有的带 aria-label（分组选择器）、有的只是个 class（开始游戏），
 * 统一成选择器能少写一个分支。
 */
function clickButtonExpression(label, scopeSelector = 'body') {
  return `(() => {
    const scope = document.querySelector(${JSON.stringify(scopeSelector)});
    if (!scope) return 'no-scope';
    const btn = [...scope.querySelectorAll('button')].find((b) => b.textContent.trim() === ${JSON.stringify(label)});
    if (!btn) return 'no-button';
    if (btn.disabled) return 'disabled';
    btn.click();
    return 'ok';
  })()`
}

/** 等某个选择器出现。 */
async function waitFor(client, selector, tries = 80) {
  for (let i = 0; i < tries; i += 1) {
    if (await evaluate(client, `!!document.querySelector(${JSON.stringify(selector)})`)) return true
    await sleep(100)
  }
  return false
}

async function main() {
  const client = await connectPage()
  console.log(`desktop_probe: 已连上 WebView2（${client.url}）`)
  await client.send('Runtime.enable')

  // 先把这个窗口归零到「刚启动」的样子：重置对局 + 回到设置页 + 刷新。
  //
  // 不这么做的话这个脚本**只能跑一次**：它连的是用户正开着的那个窗口，
  // 上一次跑完页面停在 #/play、店里还留着残局，第二次跑时 [1] 找不到设置页就失败了。
  // 桌面端没有 HTTP 接口，所以这里用 invoke + 改 hash + 刷新来复位。
  await evaluate(client, invokeExpression('new_game', { fen: null }))
  await evaluate(client, `location.hash = '#/setup'`)
  await client.send('Page.reload', {})
  await sleep(500)

  console.log('\n[1] Tauri 全局对象与页面渲染')
  // 冷启动可能被 WebView2 恢复成上次的深链（例如 #/play）。此时 store 是空的，
  // 前端会立刻把用户送回设置页 —— 但那次跳转发生在首个 effect 里，
  // 刚连上的瞬间可能还没跑完。所以**等它稳定**再断言，而不是连上就查。
  // （顺带也就验了这条兜底：没有对局时不该停在 #/play 上。）
  let settled = false
  for (let i = 0; i < 50; i += 1) {
    const now = await evaluate(
      client,
      `(document.querySelector('.setup__start') ? 'setup' : 'none') + '|' + location.hash`,
    )
    if (now === 'setup|#/setup') {
      settled = true
      break
    }
    await sleep(100)
  }
  const env = await evaluate(
    client,
    `({
      hasTauri: typeof window.__TAURI__ === 'object' && !!window.__TAURI__?.core?.invoke,
      setup: !!document.querySelector('.setup__start'),
      startLabel: document.querySelector('.setup__start .btn')?.textContent ?? '',
      hash: location.hash,
    })`,
  )
  check('window.__TAURI__.core.invoke 可用（withGlobalTauri）', env.hasTauri)
  check('冷启动（或深链兜底后）落在设置页', settled && env.hash === '#/setup', `hash=${env.hash}`)
  check('设置页有「开始游戏」', env.startLabel.includes('开始游戏'), `实际「${env.startLabel}」`)

  console.log('\n[2] 逐个命令直调（绕开前端封装）')
  const newGame = await evaluate(client, invokeExpression('new_game', { fen: null }))
  check('new_game 返回局面', newGame.ok && !!newGame.value?.state?.fen, JSON.stringify(newGame).slice(0, 160))

  const moved = await evaluate(client, invokeExpression('make_move', { from: 'h2', to: 'e2' }))
  check(
    'make_move 走出「炮二平五」',
    moved.ok && moved.value?.played?.notation === '炮二平五',
    JSON.stringify(moved).slice(0, 160),
  )

  // ⚠️ 这一条是历史崩溃点：CLI 时代走过一步再让引擎应招，会话层线程会 panic。
  //    必须**在走过一步之后**调，否则测不到。
  const engine = await evaluate(client, invokeExpression('engine_move', { level: 'l3', thinkMs: 1200 }))
  check(
    'engine_move 在已走过一步的局面下正常应招',
    engine.ok && !!engine.value?.engine?.played?.notation,
    JSON.stringify(engine).slice(0, 200),
  )
  if (engine.ok) {
    const info = engine.value.engine.info
    console.log(
      `    （引擎：${engine.value.engine.played.notation} · 深度 ${info.depth} · ${info.nodes} 节点 · ${info.think_ms} ms）`,
    )
  }

  const hint = await evaluate(client, invokeExpression('hint', { level: 'l5', thinkMs: 1200, count: 3 }))
  check(
    'hint 返回 3 条候选着法',
    hint.ok && hint.value?.hint?.suggestions?.length === 3,
    JSON.stringify(hint).slice(0, 200),
  )

  const coach = await evaluate(client, invokeExpression('coach', { level: 'l4', thinkMs: 1200 }))
  check('coach 生成战法讲解', coach.ok && !!coach.value?.note?.headline, JSON.stringify(coach).slice(0, 200))
  if (coach.ok) {
    console.log(`    （讲解：${coach.value.note.headline}）`)
  }

  const undone = await evaluate(client, invokeExpression('undo'))
  check('undo 悔棋成功', undone.ok && undone.value?.ok === true, JSON.stringify(undone).slice(0, 160))

  // ---------------------------------------------------------------- 复盘三件套
  //
  // 这三个命令只服务「终局之后看棋 / 赛后重算」，平时下棋的路径一个都不碰。
  // 所以必须在这里**单独**点名测 —— 只在真实 UI 里点，很容易永远走不到它们。
  const seekStart = await evaluate(client, invokeExpression('seek', { ply: 0 }))
  check(
    'seek 回到开局',
    seekStart.ok && seekStart.value?.state?.cursor === 0,
    JSON.stringify(seekStart).slice(0, 200),
  )
  const seekBack = await evaluate(client, invokeExpression('seek', { ply: 1 }))
  check(
    'seek 前进到第 1 手',
    seekBack.ok && seekBack.value?.state?.cursor === 1,
    JSON.stringify(seekBack).slice(0, 200),
  )
  check(
    '复盘不截断记录（还能翻回最新一手）',
    seekBack.ok && seekBack.value.state.history.length >= 1,
    `实际 ${seekBack.value?.state?.history?.length} 手`,
  )
  const seekBad = await evaluate(client, invokeExpression('seek', { ply: 999 }))
  check(
    '越界手数返回错误而不是崩溃',
    seekBad.ok === false && /不存在/.test(seekBad.error ?? ''),
    JSON.stringify(seekBad).slice(0, 160),
  )

  const analyzed = await evaluate(
    client,
    invokeExpression('analyze', { ply: 1, level: 'l4', thinkMs: 400 }),
  )
  check(
    'analyze 重算第 1 手的讲解',
    analyzed.ok && analyzed.value?.note?.ply === 1 && !!analyzed.value?.note?.headline,
    JSON.stringify(analyzed).slice(0, 200),
  )
  if (analyzed.ok) {
    console.log(`    （重算：${analyzed.value.note.headline}）`)
  }

  const badColor = await evaluate(client, invokeExpression('resign', { loser: '紫色' }))
  check(
    '非法颜色返回错误而不是崩溃',
    badColor.ok === false && /未知的颜色/.test(badColor.error ?? ''),
    JSON.stringify(badColor).slice(0, 160),
  )
  const resigned = await evaluate(client, invokeExpression('resign', { loser: 'red' }))
  check(
    'resign 认输并给出终局状态',
    resigned.ok &&
      resigned.value?.state?.status?.kind === 'resign' &&
      resigned.value.state.status.loser === 'red',
    JSON.stringify(resigned).slice(0, 240),
  )
  const afterResign = await evaluate(client, invokeExpression('make_move', { from: 'a0', to: 'a1' }))
  check(
    '终局之后落子被拒',
    afterResign.ok === false && /认输结束/.test(afterResign.error ?? ''),
    JSON.stringify(afterResign).slice(0, 160),
  )

  // ---------------------------------------------------------------- 当场结算超时
  //
  // 超时不能只在「有人试着走棋」时才发现：玩家盯着一个已经走到 0 的钟，
  // 什么都不会发生。`settle` 就是给这件事准备的，所以这里用**真的等一秒**
  // 来验它 —— 1 秒是限时配置的粒度下限，已经是能写的最短用例了。
  const shortGame = await evaluate(
    client,
    invokeExpression('new_game', {
      fen: null,
      timeControl: { base_secs: 60, step_secs: 1, byoyomi_secs: 0 },
    }),
  )
  check(
    'new_game 接受限时配置',
    shortGame.ok && shortGame.value?.state?.clock !== null,
    JSON.stringify(shortGame).slice(0, 200),
  )
  const beforeExpiry = await evaluate(client, invokeExpression('settle'))
  check(
    '没到点时 settle 不判负',
    beforeExpiry.ok && beforeExpiry.value?.state?.status?.kind === 'ongoing',
    JSON.stringify(beforeExpiry).slice(0, 200),
  )
  await sleep(1_200)
  const afterExpiry = await evaluate(client, invokeExpression('settle'))
  check(
    '到点后 settle 当场判超时',
    afterExpiry.ok &&
      afterExpiry.value?.state?.status?.kind === 'timeout' &&
      afterExpiry.value.state.status.loser === 'red',
    JSON.stringify(afterExpiry).slice(0, 240),
  )

  const bad = await evaluate(client, invokeExpression('engine_move', { level: '不存在', thinkMs: 100 }))
  check('非法难度档位返回错误而不是崩溃', bad.ok === false && /未知难度档位/.test(bad.error ?? ''), JSON.stringify(bad).slice(0, 160))

  console.log('\n[3] 真实 UI 流程：设置页开局 → 人机对战（我执黑），引擎应自动应招')
  await evaluate(client, invokeExpression('new_game', { fen: null }))
  await client.send('Page.reload', {})
  // 刷新会把 store 丢掉（`started` 回到 false），所以落地的是**设置页** ——
  // 这正是刷新兜底那条逻辑要验的东西。
  const onSetup = await waitFor(client, '.setup__start')
  check('刷新后回到设置页（不带残留对局）', onSetup === true)
  check(
    '点击「人机对战」',
    (await evaluate(client, clickButtonExpression('人机对战', '[aria-label="对局模式"]'))) === 'ok',
  )
  check(
    '点击「我执黑方」',
    (await evaluate(client, clickButtonExpression('黑方', '[aria-label="我执哪一方"]'))) === 'ok',
  )
  check(
    '点击「开始游戏」',
    (await evaluate(client, clickButtonExpression('开始游戏', '.setup__start'))) === 'ok',
  )
  check('进入对局页', (await waitFor(client, '.board')) === true)
  await sleep(300)

  let engineMoved = false
  for (let i = 0; i < 120; i += 1) {
    const state = await evaluate(client, invokeExpression('engine_state'))
    if (state.ok && (state.value?.history?.length ?? 0) >= 1) {
      engineMoved = true
      break
    }
    await sleep(200)
  }
  check('引擎在真实 UI 里自动走了一步', engineMoved, '等待 24 秒后记录仍为空')

  if (engineMoved) {
    const view = await evaluate(
      client,
      `({
        lastMarkers: document.querySelectorAll('.fx img[src*="fx-last-move"]').length,
        interactive: !document.querySelector('.board--locked'),
        onPlayPage: location.hash === '#/play',
        docScroll: document.documentElement.scrollHeight,
        winH: window.innerHeight,
      })`,
    )
    check('停在 #/play', view.onPlayPage === true)
    check('棋盘上标出了引擎走的那一步', view.lastMarkers === 2, `实际 ${view.lastMarkers}`)
    check('玩家回合棋盘可点', view.interactive)
    check('对局页没有滚动条', view.docScroll <= view.winH, `${view.docScroll} / ${view.winH}`)
  }

  const shot = await client.send('Page.captureScreenshot', { format: 'png' })
  const { writeFileSync } = await import('node:fs')
  const out = process.env.DESKTOP_SHOT ?? 'desktop-probe.png'
  writeFileSync(out, Buffer.from(shot.result.data, 'base64'))
  console.log(`\n  （收尾截图已保存：${out}）`)

  client.close()

  console.log('\n\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500')
  if (failures === 0) {
    console.log('desktop_probe: 全部通过 \u2713')
  } else {
    console.log(`desktop_probe: ${failures} 项失败 \u2717`)
    process.exit(1)
  }
}

main().catch((error) => {
  console.error('desktop_probe 运行失败：', error)
  process.exit(1)
})
