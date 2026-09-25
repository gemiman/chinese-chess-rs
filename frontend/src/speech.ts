/**
 * 语音播报。
 *
 * # 三件事分开
 *
 * 1. **素材**：`assets/voice/` 里的 mp3，由 `scripts/dev/voice/generate.py` 生成，
 *    清单在 `manifest.json`。文件名不在这里拼死 —— 拼出来的 id 查不到就跳过那一段。
 * 2. **队列**（本文件）：一步棋可能触发好几件事，得排队、得能插队、得能作废。
 * 3. **触发**（`App.tsx`）：什么时候该响由对局流程决定 —— 关键是**等棋子落下来**。
 *
 * # 最容易做砸的地方：抢话
 *
 * 一步棋常常同时是「吃车」+「将军」+「形成铁门栓」。三句一起放就是一团噪音。
 * 所以有优先级、有一手一步的作废规则：
 *
 * - **念法**：终局词 > 将军 > 吃X；三样都没有（普通棋）才报走法。
 *   战法名晚一点到（要等讲解算完），作为单独一条排在后面。
 * - **插队**：只有终局和将军能打断正在念的内容。走法播报被战法名打断会显得
 *   半句话没了，所以低优先级的只排队不插队。
 * - **作废**：新的一手一到，上一手还没念完的排队项全部丢掉 —— 欠着的账比漏报更糟。
 */

import type { CoachNote, PlayedMove, StatusDto } from './types'

const MANIFEST_URL = '/voice/manifest.json'
const CLIP_DIR = '/voice/'

/** 优先级。 */
const PRIORITY = {
  terminal: 40,
  check: 30,
  tactic: 20,
  capture: 10,
  move: 5,
} as const

/**
 * 达到这个优先级才能打断正在念的内容 —— 也就是只有终局和将军能插队。
 *
 * 走法播报（5）被战法名（20）打断的话，「红方炮二平五」会变成「红方炮二…卧槽马」，
 * 听着像卡带。所以低优先级的一律排在后面等。
 */
const INTERRUPT_AT = 30

/** 队列上限。超了从头丢 —— 排在前面的本来就是更旧的。 */
const QUEUE_LIMIT = 3

/**
 * 单个片段的兜底时长。
 *
 * `ended` 事件不是一定会来（文件坏、被系统静音、音频元素被回收），
 * 没有兜底的话一次意外就让队列永远卡在这一句上。
 */
const CLIP_GUARD_MS = 4000

/** 动作字 → 素材文件名里的拼音。 */
const ACTION_SLUG: Record<string, string> = {
  advance: 'jin',
  retreat: 'tui',
  traverse: 'ping',
}

/**
 * 这些战法名不单独念 —— 它们是**着法性质的复述**，前面已经念过了
 * （将军、吃子、绝杀都是）。真正值得单独喊的是「卧槽马」「重炮」这类棋形名。
 */
const NATURE_TACTICS = new Set([
  'check',
  'double_check',
  'discovered_check',
  'capture',
  'mate',
  'stalemate_win',
  'resolve_check',
  'interpose',
  'king_escape',
  'capture_attacker',
])

/** 战法名要有多确信才喊得出口。与 UI 里「弱样式展示」的那条线一致。 */
const TACTIC_MIN_CONFIDENCE = 0.7

interface Clip {
  file: string
  text: string
  synth: string
}

interface Line {
  ids: string[]
  priority: number
  /** 这是第几手触发的。新的一手会让旧的一手作废。 */
  turn: number
}

let clips: Record<string, Clip> | null = null
let loading: Promise<void> | null = null
const missing = new Set<string>()

let queue: Line[] = []
let audio: HTMLAudioElement | null = null
let finishClip: (() => void) | null = null
let playback = 0
let running = false

let enabled = true
let volume = 0.8
/**
 * 播放倍速。**默认比素材本身快** —— 素材按 +8% 合成，实测听着还是慢。
 *
 * 用 `playbackRate` 而不是重新合成素材：语速因此是**可调**的，用户自己拨到合适
 * 为止，不用每次让开发重录一遍。`preservesPitch` 打开，变快不变调。
 */
let speed = 1.3
/** 已经提醒过「自动播放被拦」了，别每次都刷屏。 */
let warnedAutoplay = false

/** 终局词的去重：同一局同一个终局只念一次。 */
let announcedTerminal: string | null = null

export function configure(next: { enabled: boolean; volume: number; speed: number }): void {
  enabled = next.enabled
  volume = next.volume
  speed = Math.min(2, Math.max(1, next.speed))
  if (!enabled) silence()
}

/**
 * 预热：把清单和素材取回来，让浏览器缓存住。
 *
 * 不预热的话**第一次**播报会慢半拍（要等文件下载），而第一次播报往往正是
 * 开局那几步 —— 用户会以为声音坏了。
 */
export function warmUp(): void {
  void ensureLoaded().then(async () => {
    // 同一个音只存一份文件，先去重（274 个 id 落到 209 个文件）
    const files = [...new Set(Object.values(clips ?? {}).map((clip) => clip.file))]
    // ⚠️ 分批取，**不要一次发两百个请求**：实测那样会把本地桥接服务打满，
    // 出现一批 ERR_EMPTY_RESPONSE —— 表现正是「第一句没声音」，而且只在
    // 开机后第一次播报时出现，最难查的那种。
    const lanes = 6
    for (let start = 0; start < files.length; start += lanes) {
      await Promise.all(
        files
          .slice(start, start + lanes)
          .map((file) => fetch(`${CLIP_DIR}${file}`).catch(() => undefined)),
      )
    }
  })
}

async function ensureLoaded(): Promise<void> {
  if (clips !== null) return
  loading ??= (async () => {
    const response = await fetch(MANIFEST_URL)
    const manifest = (await response.json()) as { clips: Record<string, Clip> }
    clips = manifest.clips
  })()
  await loading
}

/** 播一条棋局事件。`played` 为 `null` 表示这一步没有走子（认输、超时）。 */
export function announce(played: PlayedMove | null, status: StatusDto, turn: number): void {
  if (!enabled) return

  if (status.over) {
    // 终局只念终局词。棋都下完了，再说「吃车」「红方炮二平五」全是噪音。
    const key = `${status.kind}:${status.loser ?? ''}:${turn}`
    if (announcedTerminal === key) return
    announcedTerminal = key
    const id = terminalClip(status)
    if (id !== null) enqueue({ ids: [id], priority: PRIORITY.terminal, turn })
    return
  }

  if (played === null) return
  const lines = immediateLines(played, turn)
  for (const line of lines) enqueue(line)
}

function terminalClip(status: StatusDto): string | null {
  switch (status.kind) {
    case 'checkmate':
      return 'tactic-mate'
    case 'stalemate':
      return 'status-stalemate'
    case 'draw':
      return 'status-draw'
    case 'timeout':
      return 'status-timeout'
    case 'resign':
      return 'status-resign'
    default:
      return null
  }
}

/**
 * 这一手当场能念的内容。
 *
 * 「普通棋才报走法」是刻意的：将军、吃子、终局本身已经是信息量，再叠一句走法
 * 就变成每步都在念稿。
 */
function immediateLines(played: PlayedMove, turn: number): Line[] {
  const lines: Line[] = []

  if (played.nature === 'check') {
    lines.push({ ids: ['status-check'], priority: PRIORITY.check, turn })
  }

  if (played.captured !== null) {
    // 被吃的一定是对方的子
    const owner = played.side === 'red' ? 'black' : 'red'
    lines.push({
      ids: [`capture-${owner}-${played.captured}`],
      priority: PRIORITY.capture,
      turn,
    })
  }

  if (lines.length === 0) {
    const ids = notationIds(played)
    if (ids.length > 0) lines.push({ ids, priority: PRIORITY.move, turn })
  }

  return lines
}

/** 走法播报：棋子起点 + 动作数，两段拼一句（「炮二」+「平五」）。
 *
 * 刻意**不报「红方 / 黑方」**：两步连起来听的时候，那个词占了一半时长却不带信息
 * —— 轮到谁看棋盘和钟条就知道。省掉它，一句能短一半。
 */
function notationIds(played: PlayedMove): string[] {
  const speech = played.speech
  if (speech === null) return []

  const piece = `${played.side}-${speech.kind}`
  let wordId: string
  if (/^[1-9]$/.test(speech.subject)) {
    wordId = `word-${piece}-${speech.subject}`
  } else if (speech.subject === 'front' || speech.subject === 'back' || speech.subject === 'middle') {
    wordId = `qual-${piece}-${speech.subject}`
  } else if (speech.subject.startsWith('nth')) {
    wordId = `nth-${piece}-${speech.subject.slice(3)}`
  } else {
    return []
  }

  const action = ACTION_SLUG[speech.action]
  if (action === undefined) return []
  return [wordId, `move-${action}-${speech.value}`]
}

/**
 * 战法名。晚一步到（要等讲解算完），所以单独一条。
 *
 * `ply` 要和当前手数对上 —— 深度分析会把旧手数的讲解补回来，那不该出声。
 */
export function announceTactic(note: CoachNote, ply: number): void {
  if (!enabled) return
  const tactic = note.tactics.find((t) => t.confidence >= TACTIC_MIN_CONFIDENCE)
  if (tactic === undefined || NATURE_TACTICS.has(tactic.id)) return
  enqueue({ ids: [`tactic-${tactic.id}`], priority: PRIORITY.tactic, turn: ply })
}

/** 停下来并清空。复盘翻手、悔棋、换页、换局都该调它。 */
export function silence(): void {
  queue = []
  invalidate()
}

/**
 * 新的一局开始：把终局去重记下来。
 *
 * 去重键里带着手数，而手数每局都从 1 重新数 —— 不清的话「第 7 手被将死」这种
 * 情形在第二局会被当成已经念过，绝杀就不出声了。
 */
export function resetAnnouncements(): void {
  announcedTerminal = null
  silence()
}

function enqueue(line: Line): void {
  if (!enabled) return
  // 新的一手到了：上一手还没念完的排队项全部作废
  queue = queue.filter((queued) => queued.turn >= line.turn)

  if (audio !== null && line.priority >= INTERRUPT_AT) invalidate()

  queue.push(line)
  while (queue.length > QUEUE_LIMIT) queue.shift()
  void pump()
}

function invalidate(): void {
  playback += 1
  const element = audio
  const stop = finishClip
  if (element !== null) element.pause()
  if (stop !== null) stop()
}

async function pump(): Promise<void> {
  if (running) return
  running = true
  try {
    const pending = await ensureClips()
    if (!pending) return
    while (queue.length > 0) {
      const line = queue.shift()
      if (line === undefined) break
      const ids = resolve(line.ids)
      if (ids.length === 0) continue
      const mine = ++playback
      for (const id of ids) {
        if (mine !== playback) break
        await playClip(id)
      }
    }
  } finally {
    running = false
  }
}

async function ensureClips(): Promise<boolean> {
  try {
    await ensureLoaded()
    return true
  } catch {
    // 清单取不到（离线打开、路径变了）就安静地不播 —— 语音是锦上添花，
    // 不该因为它把对局流程带崩
    enabled = false
    return false
  }
}

/** 把查不到的 id 去掉。缺一段总比整句不播好，而且只提醒一次。 */
function resolve(ids: string[]): string[] {
  const out: string[] = []
  for (const id of ids) {
    if (clips?.[id] !== undefined) out.push(id)
    else if (!missing.has(id)) {
      missing.add(id)
      console.warn(`[voice] 没有素材 ${id}，这句播报会少一段`)
    }
  }
  return out
}

function playClip(id: string): Promise<void> {
  const clip = clips?.[id]
  if (clip === undefined) return Promise.resolve()

  return new Promise((done) => {
    const element = new Audio(`${CLIP_DIR}${clip.file}`)
    element.volume = volume
    element.playbackRate = speed
    // 变快不变调（默认就是 true，写出来是因为它正是「语速可调」能成立的前提）
    element.preservesPitch = true

    let settled = false
    let guard = 0
    const settle = () => {
      if (settled) return
      settled = true
      window.clearTimeout(guard)
      if (audio === element) audio = null
      if (finishClip === settle) finishClip = null
      done()
    }

    guard = window.setTimeout(settle, CLIP_GUARD_MS)
    element.onended = settle
    element.onerror = settle
    audio = element
    finishClip = settle
    void element.play().catch((cause: unknown) => {
      // 浏览器会拦下**没有用户手势**的自动播放。吞掉它就会变成
      // 「没声音、也没任何提示」，所以至少留一句 —— 只提醒一次。
      if (!warnedAutoplay) {
        warnedAutoplay = true
        console.warn('[voice] 浏览器拦下了自动播放，先点一下界面再试：', cause)
      }
      settle()
    })
  })
}
