/**
 * 语音播报冒烟测试 —— 在 Node 里用**假的 Audio** 跑播报逻辑，看它到底点了哪些文件。
 *
 * # 为什么不在浏览器里测
 *
 * 这一层最要命的 bug 是**拼错 id**：把 `capture-black-chariot` 写成 `capture-chariot`。
 * 它不会报错、不会崩，只是那句话**静默地少了一段** —— 而人在浏览器里听不出来是
 * 「少了一段」还是「本来就没这段」。所以要断言的是「点了哪些文件、按什么顺序」，
 * 这用假 Audio 记录最直接。
 *
 * 真跑到浏览器里反而更难断言：预热会把 200 多个 mp3 全抓一遍，网络日志里
 * 分不出哪一次是播报触发的。
 *
 * # 用法
 *
 * ```bash
 * node scripts/dev/voice_probe.mjs
 * ```
 *
 * 退出码 0 = 全部通过；1 = 有用例失败。
 */

import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const manifest = JSON.parse(readFileSync(resolve(ROOT, 'assets/voice/manifest.json'), 'utf8'))

// ------------------------------------------------------------------ 测试替身
//
// 只替三样东西：Audio（记录播放）、fetch（给清单）、window 的定时器。
// 这样跑的就是产品那份 speech.ts，不是复制品。

/** 已经被点播的文件名，按顺序。 */
let played = []
/** 每个片段播放时的倍速，用来验证语速设置真的传到了播放器。 */
let rates = []
/** 还没结束的片段。空着才能推进到下一条。 */
let held = []
/** 是否卡住当前片段（用来测排队与作废）。 */
let holding = false

class FakeAudio {
  constructor(src) {
    this.src = src
    this.onended = null
    this.onerror = null
  }

  play() {
    played.push(this.src.replace(/^\/voice\//, ''))
    rates.push(this.playbackRate)
    if (holding) {
      held.push(this)
      return Promise.resolve()
    }
    setTimeout(() => this.onended?.(), 0)
    return Promise.resolve()
  }

  pause() {}
}

function release() {
  const pending = held
  held = []
  for (const element of pending) element.onended?.()
}

globalThis.Audio = FakeAudio
globalThis.window = { setTimeout, clearTimeout }
globalThis.localStorage = {
  store: new Map(),
  getItem(key) {
    return this.store.has(key) ? this.store.get(key) : null
  },
  setItem(key, value) {
    this.store.set(key, String(value))
  },
}
globalThis.fetch = async () => ({ json: async () => manifest })

const speech = await import(pathToFileURL(resolve(ROOT, 'frontend/src/speech.ts')).href)

// ------------------------------------------------------------------ 断言
let failed = 0
let passed = 0

function check(name, actual, expected) {
  const ok = JSON.stringify(actual) === JSON.stringify(expected)
  if (ok) {
    passed += 1
    console.log(`  ✓ ${name}`)
  } else {
    failed += 1
    console.log(`  ✗ ${name}`)
    console.log(`      期望 ${JSON.stringify(expected)}`)
    console.log(`      实际 ${JSON.stringify(actual)}`)
  }
}

/**
 * 比的是**素材 id**，不是文件名。
 *
 * 同一个音只存一份文件（红车和黑车的合成文本一样），文件名取的是第一个用它的
 * id —— 所以 `qual-black-chariot-front` 实际落在 `qual-red-chariot-front.mp3` 上。
 * 这里把 id 映射成文件再比对，断言才写得出「这一步该喊哪几句」。
 */
function checkClips(name, expectedIds) {
  const expectedFiles = expectedIds.map((id) => manifest.clips[id]?.file ?? `〈缺 ${id}〉`)
  check(name, played, expectedFiles)
}

function checkThat(name, condition, detail = '') {
  if (condition) {
    passed += 1
    console.log(`  ✓ ${name}`)
  } else {
    failed += 1
    console.log(`  ✗ ${name}${detail === '' ? '' : `\n      ${detail}`}`)
  }
}

/** 等播报把该点的都点完。 */
async function drain() {
  for (let i = 0; i < 8; i += 1) await new Promise((done) => setTimeout(done, 2))
}

const IDLE = { kind: 'ongoing', text: '', over: false, loser: null }

function move(extra) {
  return {
    from: 'h2',
    to: 'e2',
    iccs: 'h2e2',
    notation: '炮二平五',
    capture: false,
    nature: 'idle',
    nature_text: '闲着',
    side: 'red',
    captured: null,
    speech: null,
    ...extra,
  }
}

function reset() {
  played = []
  rates = []
  held = []
  holding = false
  speech.resetAnnouncements()
}

// ------------------------------------------------------------------ 用例
console.log('\n[1] 吃子只喊被吃的那个子')
reset()
speech.announce(move({ capture: true, captured: 'chariot', nature: 'capture' }), IDLE, 1)
await drain()
checkClips('吃掉黑车 → 喊「吃驹」', ['capture-black-chariot'])

reset()
speech.announce(move({ capture: true, captured: 'king', nature: 'capture' }), IDLE, 1)
await drain()
checkClips('吃掉黑将 → 用「匠」那条素材', ['capture-black-king'])

console.log('\n[2] 将军与战术名分开、按顺序')
reset()
speech.announce(move({ nature: 'check' }), IDLE, 1)
await drain()
checkClips('将军 → status-check', ['status-check'])

reset()
speech.announce(
  move({ nature: 'check' }),
  IDLE,
  1,
)
speech.announceTactic(
  { ply: 1, tactics: [{ id: 'horse_slot_check', name: '卧槽马', category: 'formation', confidence: 0.9 }] },
  1,
)
await drain()
checkClips('将军 + 卧槽马 → 两句，将军在前', ['status-check', 'tactic-horse_slot_check'])

reset()
speech.announceTactic(
  { ply: 1, tactics: [{ id: 'check', name: '将军', category: 'structure', confidence: 0.99 }] },
  1,
)
await drain()
check('战法名是「将军」时不重复喊', played, [])

reset()
speech.announceTactic(
  { ply: 1, tactics: [{ id: 'pin', name: '牵制', category: 'relation', confidence: 0.4 }] },
  1,
)
await drain()
check('把握不足的战法名不喊（0.4 < 0.7）', played, [])

console.log('\n[3] 普通棋才报走法（三段拼接）')
reset()
speech.announce(
  move({ speech: { kind: 'cannon', subject: '2', action: 'traverse', value: 5 } }),
  IDLE,
  1,
)
await drain()
checkClips('炮二平五（不报红黑方）', ['word-red-cannon-2', 'move-ping-5'])

reset()
speech.announce(move({ nature: 'check', speech: { kind: 'cannon', subject: '2', action: 'traverse', value: 5 } }), IDLE, 1)
await drain()
checkClips('将军的一手不再叠走法', ['status-check'])

reset()
speech.announce(
  move({ speech: { kind: 'chariot', subject: 'front', action: 'advance', value: 1 }, side: 'black' }),
  IDLE,
  1,
)
await drain()
checkClips('同线两子：前车进一', ['qual-black-chariot-front', 'move-jin-1'])

reset()
speech.announce(
  move({ speech: { kind: 'pawn', subject: 'nth3', action: 'advance', value: 1 }, side: 'black' }),
  IDLE,
  1,
)
await drain()
checkClips('同线四卒：三卒进一', ['nth-black-pawn-3', 'move-jin-1'])

console.log('\n[4] 终局只念终局词')
reset()
speech.announce(
  move({ nature: 'check', capture: true, captured: 'chariot' }),
  { kind: 'checkmate', text: '将死', over: true, loser: 'black' },
  7,
)
speech.announce(
  move({ nature: 'check', capture: true, captured: 'chariot' }),
  { kind: 'checkmate', text: '将死', over: true, loser: 'black' },
  7,
)
await drain()
checkClips('将死 → 只喊绝杀，且不重复', ['tactic-mate'])

reset()
speech.announce(null, { kind: 'timeout', text: '超时', over: true, loser: 'red' }, 12)
await drain()
checkClips('超时（没有走子）→ status-timeout', ['status-timeout'])

reset()
speech.announce(null, { kind: 'resign', text: '认输', over: true, loser: 'red' }, 9)
await drain()
checkClips('认输 → status-resign', ['status-resign'])

console.log('\n[5] 抢话：只有终局和将军能打断')
reset()
holding = true
speech.announce(move({ nature: 'check' }), IDLE, 1)
await drain()
check('将军开始播', played, ['status-check.mp3'])

speech.announce(
  move({ speech: { kind: 'horse', subject: '8', action: 'advance', value: 7 }, side: 'black' }),
  IDLE,
  2,
)
held = []
holding = false
release()
await drain()
checkThat(
  '第 2 手的走法没把将军掐断',
  played[0] === 'status-check.mp3',
  `实际顺序 ${JSON.stringify(played)}`,
)

reset()
holding = true
speech.announce(
  move({ speech: { kind: 'horse', subject: '8', action: 'advance', value: 7 }, side: 'black' }),
  IDLE,
  1,
)
await drain()
speech.announce(move({ nature: 'check' }), IDLE, 2)
held = []
holding = false
release()
await drain()
checkThat(
  '将军能打断正在念的走法',
  played[0] === manifest.clips['word-black-horse-8'].file &&
    played.includes('status-check.mp3'),
  `实际顺序 ${JSON.stringify(played)}`,
)

console.log('\n[6] 新的一手作废旧排队项')
reset()
holding = true
speech.announce(move({ nature: 'check' }), IDLE, 1)
await drain()
speech.announceTactic(
  { ply: 1, tactics: [{ id: 'pin', name: '牵制', category: 'relation', confidence: 0.9 }] },
  1,
)
// 第 2 手来了：第 1 手排在队里的战法名该被丢掉
speech.announce(move({ speech: { kind: 'cannon', subject: '2', action: 'traverse', value: 5 } }), IDLE, 2)
held = []
holding = false
release()
await drain()
checkThat(
  '第 1 手的战法名没跟着念出来',
  !played.includes('tactic-pin.mp3'),
  `实际顺序 ${JSON.stringify(played)}`,
)

console.log('\n[7] 缺素材只跳过那一段，不崩')
reset()
speech.announce(
  move({ speech: { kind: 'cannon', subject: '9', action: 'retreat', value: 3 }, side: 'black' }),
  IDLE,
  1,
)
await drain()
checkClips('两段都在时不缺段', ['word-black-cannon-9', 'move-tui-3'])

console.log('\n[8] 清单覆盖：播报逻辑可能拼出的 id 全都要有素材')
const colors = ['red', 'black']
const kinds = ['chariot', 'horse', 'cannon', 'elephant', 'advisor', 'king', 'pawn']
const asked = new Set()
for (const color of colors) {
  // 不列 side-*：播报已经不报红黑方了，那两个词条也一并从词表里去掉
  for (const kind of kinds) {
    asked.add(`capture-${color}-${kind}`)
    for (let n = 1; n <= 9; n += 1) asked.add(`word-${color}-${kind}-${n}`)
    for (const subject of ['front', 'back', 'middle']) asked.add(`qual-${color}-${kind}-${subject}`)
  }
  // 序数只给兵/卒生成：同一条竖线上凑够 4 个同种同色棋子，只有兵卒做得到
  for (let n = 2; n <= 8; n += 1) asked.add(`nth-${color}-pawn-${n}`)
}
for (const action of ['jin', 'tui', 'ping']) {
  for (let n = 1; n <= 9; n += 1) asked.add(`move-${action}-${n}`)
}
for (const id of [
  'status-check',
  'status-stalemate',
  'status-draw',
  'status-resign',
  'status-timeout',
  'tactic-mate',
]) {
  asked.add(id)
}

const absent = [...asked].filter((id) => manifest.clips[id] === undefined)
checkThat(
  `${asked.size} 个可能拼出的 id 都在清单里`,
  absent.length === 0,
  absent.length === 0 ? '' : `缺 ${absent.length} 个：${absent.slice(0, 8).join('、')}`,
)

speech.configure({ enabled: true, volume: 0.8, speed: 1.3 })
const tactics = Object.keys(manifest.clips).filter((id) => id.startsWith('tactic-'))
checkThat(`战术名素材有 ${tactics.length} 条`, tactics.length >= 45)

console.log('\n[9] 语速设置真的传到了播放器')
reset()
speech.configure({ enabled: true, volume: 0.8, speed: 1.6 })
speech.announce(move({ nature: 'check' }), IDLE, 1)
await drain()
check('1.6× 下播的片段按 1.6 播', rates, [1.6])
speech.configure({ enabled: true, volume: 0.8, speed: 9 })
reset()
speech.announce(move({ nature: 'check' }), IDLE, 2)
await drain()
check('超范围的语速被夹到 2', rates, [2])

// ------------------------------------------------------------------ 收尾
console.log(`\n通过 ${passed} 项，失败 ${failed} 项`)
process.exit(failed === 0 ? 0 : 1)
