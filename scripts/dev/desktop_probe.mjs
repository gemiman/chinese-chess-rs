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

/** 按文字点击一个按钮，返回是否找到并点中。 */
function clickButtonExpression(label, ariaLabel) {
  return `(() => {
    const scope = ${ariaLabel ? `document.querySelector(${JSON.stringify(`[aria-label="${ariaLabel}"]`)})` : 'document'};
    if (!scope) return 'no-group';
    const btn = [...scope.querySelectorAll('button')].find((b) => b.textContent.trim() === ${JSON.stringify(label)});
    if (!btn) return 'no-button';
    if (btn.disabled) return 'disabled';
    btn.click();
    return 'ok';
  })()`
}

async function main() {
  const client = await connectPage()
  console.log(`desktop_probe: 已连上 WebView2（${client.url}）`)
  await client.send('Runtime.enable')

  console.log('\n[1] Tauri 全局对象与页面渲染')
  const env = await evaluate(
    client,
    `({
      hasTauri: typeof window.__TAURI__ === 'object' && !!window.__TAURI__?.core?.invoke,
      board: !!document.querySelector('.board'),
      pieces: document.querySelectorAll('.piece').length,
    })`,
  )
  check('window.__TAURI__.core.invoke 可用（withGlobalTauri）', env.hasTauri)
  check('页面已渲染棋盘', env.board)
  check('棋子 32 枚', env.pieces === 32, `实际 ${env.pieces}`)

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

  const bad = await evaluate(client, invokeExpression('engine_move', { level: '不存在', thinkMs: 100 }))
  check('非法难度档位返回错误而不是崩溃', bad.ok === false && /未知难度档位/.test(bad.error ?? ''), JSON.stringify(bad).slice(0, 160))

  console.log('\n[3] 真实 UI 流程：人机对战（我执黑），引擎应自动应招')
  await evaluate(client, invokeExpression('new_game', { fen: null }))
  await client.send('Page.reload', {})
  for (let i = 0; i < 80; i += 1) {
    if (await evaluate(client, `!!document.querySelector('.board')`)) break
    await sleep(100)
  }
  await sleep(300)

  check('点击「人机对战」', (await evaluate(client, clickButtonExpression('人机对战', '对局模式'))) === 'ok')
  check('点击「我执黑方」', (await evaluate(client, clickButtonExpression('黑方', '我执哪一方'))) === 'ok')

  let engineMoved = false
  for (let i = 0; i < 120; i += 1) {
    if ((await evaluate(client, `document.querySelectorAll('.moves__row').length`)) >= 1) {
      engineMoved = true
      break
    }
    await sleep(200)
  }
  check('引擎在真实 UI 里自动走了一步', engineMoved, '等待 24 秒后记谱仍为空')

  if (engineMoved) {
    const view = await evaluate(
      client,
      `({
        firstRed: document.querySelector('.moves__red')?.textContent ?? '',
        info: document.querySelector('.engine-line')?.textContent ?? '',
        interactive: !document.querySelector('.board--locked'),
      })`,
    )
    check('记谱里有红方第一着', view.firstRed.length > 0, `实际「${view.firstRed}」`)
    check('显示了引擎搜索信息', /深度\s*\d+/.test(view.info), `实际「${view.info.trim()}」`)
    check('玩家回合棋盘可点', view.interactive)
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
