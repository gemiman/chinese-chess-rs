/**
 * 生成 README 用的功能演示截图。
 *
 * # 为什么要写成脚本而不是手工截
 *
 * 手工截的图**会过时**：界面一改，README 里那张图就跟不上，而且没人会想起来重截。
 * 写成脚本之后，界面改完重跑一次就好，步骤也是可复现的。
 *
 * # 为什么用无头 Chrome 而不是截桌面端窗口
 *
 * 前端是同一份，浏览器里截出来和桌面端长得一样，而且能**指定视口尺寸** ——
 * 这里用 1380×940，与 `tauri.conf.json` 里桌面窗口的尺寸一致，所以截出来的
 * 就是用户在桌面端看到的样子。截真实窗口还得处理窗口位置、DPI、遮挡，不值当。
 *
 * # 图片放哪
 *
 * `docs/images/`，**不能放 `assets/`** —— 那是 vite 的 publicDir，放进去会被打包进
 * 应用安装包（几张截图白占几百 KB，而且用户根本看不到）。
 *
 * # 用法
 *
 * ```bash
 * # 1. 起带调试端口的无头 Chrome（见 ui_probe.mjs 的用法说明）
 * # 2. 起桥接服务：cargo run -p xq-bridge
 * node scripts/dev/shots.mjs
 * ```
 *
 * 退出码 0 = 截图齐了；1 = 有场景没截成。
 */

import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const OUT_DIR = resolve(ROOT, 'docs/images')

const CDP_PORT = Number(process.env.CDP_PORT ?? 9222)
const PAGE_URL = process.argv[2] ?? 'http://127.0.0.1:8848/'
const CDP = `http://127.0.0.1:${CDP_PORT}`
const ORIGIN = new URL(PAGE_URL).origin

/** 与 `crates/xq-client/tauri.conf.json` 里桌面窗口的尺寸保持一致。 */
const VIEWPORT = { width: 1380, height: 940 }

/** 棋盘上「一步将死」的局面：红俥九平五，马控一角、双俥封门，黑方无子可垫。 */
const MATE_FEN = '4k4/9/6N2/9/9/9/9/9/R8/3R1K3 w - - 0 1'

let failures = 0
const written = []

async function openTarget(url) {
  let res = await fetch(`${CDP}/json/new?${encodeURIComponent(url)}`, { method: 'PUT' })
  if (!res.ok) res = await fetch(`${CDP}/json/new?${encodeURIComponent(url)}`)
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
    const done = pending.get(message.id)
    if (done) {
      pending.delete(message.id)
      done(message)
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

const sleep = (ms) => new Promise((done) => setTimeout(done, ms))

async function waitFor(client, selector, tries = 60) {
  for (let i = 0; i < tries; i += 1) {
    if (await evaluate(client, `!!document.querySelector(${JSON.stringify(selector)})`)) return true
    await sleep(200)
  }
  return false
}

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

async function startGame(client) {
  const ok = await waitFor(client, '.setup__start')
  if (!ok) return 'no-setup'
  for (let i = 0; i < 40; i += 1) {
    const state = await clickButton(client, '.setup__start', '开始游戏')
    if (state === 'ok') {
      await sleep(1200)
      return 'ok'
    }
    await sleep(250)
  }
  return 'busy'
}

/** 用记谱输入框走一步（走子由服务端裁决，所以不受本页局部盘面影响）。 */
async function playText(client, text) {
  await clickButton(client, '.play__bar', '记谱')
  await sleep(250)
  const typed = await evaluate(
    client,
    `(() => {
      const input = document.querySelector('#notation-input');
      if (!input) return false;
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
      setter.call(input, ${JSON.stringify(text)});
      input.dispatchEvent(new Event('input', { bubbles: true }));
      return true;
    })()`,
  )
  if (!typed) return 'no-input'
  await sleep(150)
  await clickButton(client, '.sheet', '走')
  await sleep(1100)
  return 'ok'
}

async function shoot(client, name, { waitMs = 0 } = {}) {
  if (waitMs > 0) await sleep(waitMs)
  const reply = await client.send('Page.captureScreenshot', { format: 'png' })
  const data = reply.result?.data
  if (!data) {
    failures += 1
    console.log(`  ✗ ${name}：截图没拿到数据`)
    return
  }
  const path = resolve(OUT_DIR, `${name}.png`)
  writeFileSync(path, Buffer.from(data, 'base64'))
  const kb = Math.round(Buffer.from(data, 'base64').length / 1024)
  written.push({ name, kb })
  console.log(`  ✓ ${name}.png  ${kb} KB`)
}

async function resetGame() {
  await fetch(`${ORIGIN}/api/new`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: '{}',
  }).catch(() => undefined)
}

/** 一段真实的开局着法（红黑交替，逐步合法），用来截「有内容」的对局与分析页。 */
const OPENING = [
  'h2e2', // 炮二平五
  'h7e7', // 砲8平5
  'h0g2', // 傌二进三
  'h9g7', // 馬8进7
  'i0h0', // 俥一平二
  'i9h9', // 車9平8
  'g3g4', // 兵三进一
  'g6g5', // 卒7进1
  'b0c2', // 傌八进七
  'b9c7', // 馬2进3
  'c3c4', // 兵七进一
  'c6c5', // 卒3进1
]

async function main() {
  mkdirSync(OUT_DIR, { recursive: true })

  const client = await openTarget('about:blank')
  // 视口必须在导航**之前**设好：棋盘尺寸是按视口高度反推的，
  // 先加载再改视口会拿到一帧按旧尺寸排好的版。
  await client.send('Emulation.setDeviceMetricsOverride', {
    ...VIEWPORT,
    deviceScaleFactor: 1,
    mobile: false,
  })
  await client.send('Page.navigate', { url: PAGE_URL.replace(/#.*$/, '') })
  await waitFor(client, '.setup__start')

  console.log('\n[1] 设置页（三种对局模式 + 语音播报）')
  await shoot(client, '01-setup', { waitMs: 600 })

  console.log('\n[2] 对局页（棋盘 + 两条钟 + 走子标记）')
  if ((await startGame(client)) !== 'ok') {
    failures += 1
    console.log('  ✗ 开局失败')
  }
  for (const move of OPENING) await playText(client, move)
  await shoot(client, '02-play', { waitMs: 800 })

  console.log('\n[3] 终局分析页（结果 + 统计 + 逐手讲解）')
  // 先等讲解把这 12 手补完，否则「已讲解」是个没追上的数字，截图会误导人
  await sleep(12000)
  await clickButton(client, '.play__bar', '认输')
  await sleep(400)
  await clickButton(client, '.sheet', '确认认输')
  let arrived = false
  for (let i = 0; i < 60; i += 1) {
    if ((await evaluate(client, 'location.hash')) === '#/analysis') {
      arrived = true
      break
    }
    await sleep(250)
  }
  if (!arrived) {
    failures += 1
    console.log('  ✗ 认输后没有跳到分析页')
  }
  await shoot(client, '04-analysis', { waitMs: 1200 })

  console.log('\n[4] 战法名出场（绝杀）')
  // 服务端摆好「一步将死」的局面，本页照旧用记谱走最后一步 —— 走子由服务端裁决
  await evaluate(client, `location.hash = '#/setup'`)
  await waitFor(client, '.setup__start')
  if ((await startGame(client)) !== 'ok') {
    failures += 1
    console.log('  ✗ 开局失败')
  }
  await fetch(`${ORIGIN}/api/new`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ fen: MATE_FEN }),
  })
  await playText(client, 'a1e1')
  // 牌匾只挂 2.5 秒，所以要轮询抓到它出现的那一刻
  let caught = false
  for (let i = 0; i < 30; i += 1) {
    if (await evaluate(client, `!!document.querySelector('.tactic')`)) {
      caught = true
      break
    }
    await sleep(100)
  }
  if (!caught) {
    failures += 1
    console.log('  ✗ 没等到出场特效（牌匾可能已经收起来了）')
  }
  await shoot(client, '03-mate')

  console.log('\n[5] 机机对战（AI 档位 + 战绩 + 叫停按钮）')
  await evaluate(client, `location.hash = '#/setup'`)
  await waitFor(client, '.setup__start')
  await clickButton(client, '[aria-label="对局模式"]', '机机对战')
  await sleep(300)
  if ((await startGame(client)) !== 'ok') {
    failures += 1
    console.log('  ✗ 机机对战开局失败')
  }
  // 等两边各走几步：钟上会出现档位标记、下满一手会出战绩
  await sleep(9000)
  await shoot(client, '05-auto')

  // 收尾：把局面复位，别把一局走到一半的棋留给下一次运行或用户
  await resetGame()
  await fetch(`${CDP}/json/close/${client.targetId}`).catch(() => undefined)
  client.close()

  const total = written.reduce((sum, item) => sum + item.kb, 0)
  console.log(`\n────────────────────────────────`)
  console.log(`共 ${written.length} 张，合计 ${total} KB，在 docs/images/`)
  if (failures > 0) {
    console.log(`shots: ${failures} 个场景失败 ✗`)
    process.exit(1)
  }
  console.log('shots: 全部成功 ✓')
}

await main()
