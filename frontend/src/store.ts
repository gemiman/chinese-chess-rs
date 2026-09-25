/**
 * 对局状态（Zustand）。
 *
 * 设计取舍：**局面真相在 Rust 侧**，前端只保存最近一次快照。
 * 所有走子都发到引擎、由引擎裁决后再回填状态。
 *
 * 为什么不乐观更新：M1 是本地单机，一次请求往返在微秒级，
 * 乐观更新带来的复杂度（回滚、竞态、版本号）换不到任何体感收益。
 * 联网对战（M2）才需要 ADR-003 的乐观更新 + 回滚。
 */

import { create } from 'zustand'

import { bridge } from './bridge'
import { configure, resetAnnouncements, warmUp } from './speech'
import {
  AUTO_GAP_MS,
  DEEP_LEVEL,
  DEEP_THINK_MS,
  DIFFICULTIES,
  THINK_MS,
  type CoachNote,
  type Color,
  type DifficultyId,
  type EngineInfo,
  type GameMode,
  type HintItem,
  type MoveOption,
  type MoveSpeed,
  type StateDto,
  type TimePresetId,
} from './types'
import { timePresetOf } from './types'

/**
 * 「引擎正在思考」的同步守卫。
 *
 * 为什么不只用 store 里的 `thinking` 字段：React 18 的 StrictMode 会把副作用
 * 跑两遍，而 `set()` 之后组件重渲染需要一次微任务 —— 两次调用之间 `thinking`
 * 还是 `false`，于是会发出**两次**引擎请求，走出两步棋。
 * 用一个模块级变量做同步短路最稳。
 */
let engineInFlight = false

/**
 * 已经被「深度分析」重算过的手数。
 *
 * 放在模块级而不是 store 里：它是**簿记**不是界面状态，没人订阅它。
 * 有了它，重算中途取消后再点「继续」就不必从第一手重来 —— 一局棋几十手，
 * 每手几百毫秒，从头再来一次是要等出火的。
 */
const deepDone = new Set<number>()

/** 「正在结算超时」的同步守卫。见 `settleTimeout`。 */
let settlingTimeout = false

/**
 * 「正在开下一局」的同步守卫。
 *
 * 与 `engineInFlight` 同一个理由：StrictMode 会把副作用跑两遍，而 `startNextGame`
 * 是异步的 —— 两次调用会**连着开两局**（第二局把刚开的那个覆盖掉）。
 * 界面上看不出报错，只觉得「怎么闪了一下又是新棋」。
 */
let startingNext = false

/** 机机对战的累计战绩。 */
export interface AutoRecord {
  red: number
  black: number
  draw: number
}

const EMPTY_RECORD: AutoRecord = { red: 0, black: 0, draw: 0 }

/**
 * 给双方各摇一档。只摇**没被手动选过**的那一方。
 *
 * 「各自随机」是分开摇的，不是摇一次给两边用 —— 那样只会出现「同档对同档」，
 * 看不到大师打入门这种最有意思的对局。
 */
function rollAutoLevels(
  current: Record<Color, DifficultyId>,
  chosen: Record<Color, boolean>,
): Record<Color, DifficultyId> {
  const pick = () => DIFFICULTIES[Math.floor(Math.random() * DIFFICULTIES.length)].id
  return {
    red: chosen.red ? current.red : pick(),
    black: chosen.black ? current.black : pick(),
  }
}

/** 深度分析的初始进度。 */
const IDLE_DEEP: DeepProgress = { running: false, done: 0, total: 0, error: null }

/** 赛后深度分析的进度。 */
export interface DeepProgress {
  running: boolean
  /** 已覆盖的手数（跨多次运行累计）。 */
  done: number
  /** 一共多少手。 */
  total: number
  error: string | null
}

/** 走子动画速度的本地存储键。 */
const SPEED_KEY = 'xq.move-speed'

/** 读回上次选择的速度档位；存不了或值非法时回落到 normal。 */
function loadSpeed(): MoveSpeed {
  try {
    const raw = localStorage.getItem(SPEED_KEY)
    if (raw === 'fast' || raw === 'normal' || raw === 'slow') return raw
  } catch {
    // 隐私模式下 localStorage 可能被禁用，回落默认值即可，不必打扰用户
  }
  return 'normal'
}

/** 记住速度选择。存不上也不影响本次会话。 */
function saveSpeed(speed: MoveSpeed): void {
  try {
    localStorage.setItem(SPEED_KEY, speed)
  } catch {
    // 同上
  }
}

/**
 * 这一手该用哪个档位。
 *
 * 机机对战两边各有一档（能看「大师 vs 入门」这种让子局）；人机对战只有玩家
 * 对面那一侧会用到引擎，两处共用一个档位。
 */
function levelForSide(
  mode: GameMode,
  side: Color,
  difficulty: DifficultyId,
  autoLevels: Record<Color, DifficultyId>,
): DifficultyId {
  return mode === 'auto' ? autoLevels[side] : difficulty
}

/** 限时档位的本地存储键。 */
const PRESET_KEY = 'xq.time-preset'

function isPresetId(v: string): v is TimePresetId {
  return v === 'none' || v === 'blitz' || v === 'standard' || v === 'slow'
}

/** 读回上次选择的时间档位；存不了或值非法时回落到「不限时」。 */
function loadPreset(): TimePresetId {
  try {
    const raw = localStorage.getItem(PRESET_KEY)
    if (raw !== null && isPresetId(raw)) return raw
  } catch {
    // 同 loadSpeed
  }
  return 'none'
}

function savePreset(preset: TimePresetId): void {
  try {
    localStorage.setItem(PRESET_KEY, preset)
  } catch {
    // 同上
  }
}

/** 语音开关的本地存储键。 */
const VOICE_KEY = 'xq.voice-enabled'
/** 语音音量的本地存储键。 */
const VOICE_VOLUME_KEY = 'xq.voice-volume'
/** 语速倍率的本地存储键。 */
const VOICE_SPEED_KEY = 'xq.voice-speed'

/**
 * 语速默认 1.3 —— **比素材本身快**。
 *
 * 素材是按 +8% 合成的，实测听着仍然慢；与其重录一遍素材，不如让播放端可调，
 * 用户自己拨到合适为止（见 `speech.ts` 的 `configure`）。
 */
function loadVoiceSpeed(): number {
  try {
    const raw = localStorage.getItem(VOICE_SPEED_KEY)
    if (raw !== null) {
      const value = Number(raw)
      if (Number.isFinite(value) && value >= 1 && value <= 2) return value
    }
  } catch {
    // 同 loadSpeed
  }
  return 1.3
}

function saveVoiceSpeed(speed: number): void {
  try {
    localStorage.setItem(VOICE_SPEED_KEY, String(speed))
  } catch {
    // 同上
  }
}

/**
 * 语音默认**开着** —— 这是用户要的功能，默认关掉等于没做。
 *
 * 音量默认 0.8 而不是 1.0：喊「将军」「绝杀」的片段本身录得比较冲，
 * 满音量会吓人一跳。
 */
function loadVoiceEnabled(): boolean {
  try {
    return localStorage.getItem(VOICE_KEY) !== 'off'
  } catch {
    return true
  }
}

function loadVoiceVolume(): number {
  try {
    const raw = localStorage.getItem(VOICE_VOLUME_KEY)
    if (raw !== null) {
      const value = Number(raw)
      if (Number.isFinite(value) && value >= 0 && value <= 1) return value
    }
  } catch {
    // 同 loadSpeed
  }
  return 0.8
}

function saveVoiceEnabled(enabled: boolean): void {
  try {
    localStorage.setItem(VOICE_KEY, enabled ? 'on' : 'off')
  } catch {
    // 同 loadSpeed
  }
}

function saveVoiceVolume(volume: number): void {
  try {
    localStorage.setItem(VOICE_VOLUME_KEY, String(volume))
  } catch {
    // 同上
  }
}

/**
 * 操作被拒绝之后重新拉一次权威局面。
 *
 * 最典型的是**超时判负**：Rust 是在落子那一刻才发现表走完了 —— 它把这一步拒掉、
 * 记下终局，但**拒绝的响应里没有局面**（错误响应只有一条消息）。不重拉的话，
 * 前端会停在一个「看着还能走、其实已经结束」的盘面上：既不跳分析页，
 * 也不显示谁超时了，用户只能反复点、反复吃同一个错误。
 *
 * 只在出错时调用，正常路径一次多余的请求都没有。
 */
async function refreshAfterFailure(fallback: StateDto | null): Promise<StateDto | null> {
  try {
    return await bridge.state()
  } catch {
    // 连局面都拉不到（服务挂了），那就只剩错误可报
    return fallback
  }
}

interface GameStore {
  state: StateDto | null
  /**
   * 是否已经点过「开始游戏」。
   *
   * 光看 `state !== null` 判断不了 —— 开机就会拉一次局面，那时用户还在设置页。
   * 有了它，刷新后直接落在 `#/play`、`#/analysis` 时才知道应该送回设置页。
   */
  started: boolean
  /** 当前选中的棋子（ICCS 坐标）；未选中为 `null`。 */
  selected: string | null
  error: string | null
  /** 是否正在等待引擎响应。 */
  busy: boolean
  /** 是否翻转视角（执黑时用）。 */
  flipped: boolean
  /** 走子动画速度档位（记在 localStorage 里）。 */
  moveSpeed: MoveSpeed
  /** 限时档位（记在 localStorage 里）。开局时下发给 Rust。 */
  timePreset: TimePresetId

  /** 对局模式：双人同机 / 人机对战 / 机机对战。 */
  mode: GameMode
  /** 人机模式下玩家执哪一方。 */
  playerColor: Color
  /** 人机模式下的引擎档位。 */
  difficulty: DifficultyId
  /** 机机对战两边的档位，红黑各自一档。 */
  autoLevels: Record<Color, DifficultyId>
  /**
   * 机机对战：哪几方的档位是**手动选过**的。
   *
   * 没选过的一方每开一局都重摇 —— 「各自随机」是按方计的，
   * 只手动定了一边时，另一边照样每局换人。
   */
  autoLevelsChosen: Record<Color, boolean>
  /**
   * 机机对战是否连播。
   *
   * 暂停时两个 AI 都停手、棋盘只读；点「单步」让当前该走的那一方走一步。
   * 看 AI 对局时这一步很要紧 —— 一步两三秒，看到关键处总得能停下来想想。
   */
  autoPlaying: boolean
  /**
   * 叫停机机对战的连播。
   *
   * 与「暂停」不同：暂停只是把倒计时按住，按「继续」就接着跑；叫停是把连播**彻底结束** ——
   * 待开的下一局也清掉，用户回到开局前的设置页。看几十局之后想收手，只能暂停是不够的。
   */
  stopAuto: () => void
  /**
   * 机机对战：下一局的开始时刻（毫秒时间戳）；`null` = 没排下一局。
   *
   * 存**时刻**而不是「还剩几秒」：秒数要有人每秒去改它，而时刻是死的，
   * 谁需要谁拿 `Date.now()` 减一下就行。暂停期间它不动，继续的时候接着算，
   * 语义天然正确。
   */
  nextGameAt: number | null
  /** 机机对战的累计战绩（这一轮连播里红方赢了几局、黑方赢了几局、和了几局）。 */
  autoRecord: AutoRecord
  /** 引擎是否正在思考。 */
  thinking: boolean
  /** 最近一次引擎搜索的元信息。 */
  engineInfo: EngineInfo | null
  /** 走棋提示（按需请求）。 */
  hints: HintItem[]
  hintInfo: EngineInfo | null

  /** 最后一步的战法讲解（对局页出场特效与卡片用）。 */
  coachNote: CoachNote | null
  coachInfo: EngineInfo | null
  /** 讲解生成中（比走子慢，因为它要跑一次搜索）。 */
  coachLoading: boolean
  /**
   * **整局**的战法讲解，按手数存。
   *
   * 走一步攒一条，不再被下一步覆盖 —— 终局分析页要把几十手摆出来看。
   * 键是**手数**（1 起），与 `CoachNote.ply` 一致。
   */
  notes: Record<number, CoachNote>
  /** 赛后深度分析的进度。 */
  deep: DeepProgress

  /** 拉取一次局面。 */
  load: () => Promise<void>
  /** 点击某个交叉点。选中 / 落子 / 取消 三态由这里统一决定。 */
  clickSquare: (sq: string) => void
  /** 清空选中。 */
  clearSelection: () => void
  /** 按记谱或坐标走一步（供输入框使用）。 */
  playText: (text: string) => Promise<void>
  /** 直接走一步（供推荐着法条使用）。 */
  playMove: (from: string, to: string) => Promise<void>
  /** 悔一步。人机模式下会连退两步，回到玩家自己的回合。 */
  undo: () => Promise<void>
  /** 按当前设置开一局新的。设置页的「开始游戏」用这个。 */
  startGame: () => Promise<void>
  /** 切换视角。 */
  toggleFlip: () => void
  /** 设置走子动画速度。 */
  setMoveSpeed: (speed: MoveSpeed) => void
  /** 设置限时档位。下一局开局时生效。 */
  setTimePreset: (preset: TimePresetId) => void
  /** 语音播报开关（记在 localStorage 里）。 */
  voiceEnabled: boolean
  /** 语音音量 0~1（记在 localStorage 里）。 */
  voiceVolume: number
  /** 语速倍率 1~2（记在 localStorage 里）。 */
  voiceSpeed: number
  /** 开关语音播报。 */
  toggleVoice: () => void
  /** 设置语音音量。 */
  setVoiceVolume: (volume: number) => void
  /** 设置语速倍率。 */
  setVoiceSpeed: (speed: number) => void
  /** 手动清除错误提示。 */
  dismissError: () => void

  setMode: (mode: GameMode) => void
  setPlayerColor: (color: Color) => void
  setDifficulty: (level: DifficultyId) => void
  /**
   * 设置机机对战某一方的档位。传 `null` 表示「随机」——
   * 该方从此每局重摇，不再固定。
   */
  setAutoLevel: (color: Color, level: DifficultyId | null) => void
  /** 机机对战：暂停 / 继续。终局时「继续」= 立刻开下一局。 */
  toggleAutoPlaying: () => void
  /**
   * 机机对战：只走一步。
   *
   * 顺带把连播关掉 —— 不关的话走完这一步副作用会立刻接着走下一步，
   * 「单步」就等于没按。
   */
  stepOnce: () => Promise<void>
  /** 机机对战：排下一局（设置 `nextGameAt`）。 */
  armNextGame: () => void
  /** 机机对战：立刻开下一局（跳过倒计时）。 */
  startNextGame: () => Promise<void>
  /** 机机对战：一局分出结果了，记一笔战绩。 */
  finishAutoGame: () => void
  /** 让引擎走一步（由 App 的副作用在轮到引擎时调用）。 */
  enginePlay: () => Promise<void>
  /** 请求走棋提示。 */
  requestHint: () => Promise<void>
  /** 清掉提示。 */
  clearHint: () => void

  /** 请求最后一步的战法讲解。 */
  requestCoach: () => Promise<void>
  /** 清掉讲解（悔棋 / 重开时）。 */
  clearCoach: () => void

  // ---------------------------------------------------------------- 复盘与终局

  /**
   * 自己认输。
   *
   * 认输方由这里决定，不让页面传：人机对战认输的当然是玩家（哪怕此刻轮到
   * 引擎思考）；双人同机则是**当前该走的那一方**认输 —— 谁盯着棋盘就是谁。
   */
  resign: () => Promise<void>
  /** 复盘：把盘面挪到第 `ply` 手之后（`0` = 开局）。 */
  seek: (ply: number) => Promise<void>
  /**
   * 当场结算超时。
   *
   * 由棋钟在自己的步时归零时调用。**超时不能只在「有人试着走棋」时才发现** ——
   * 那样玩家盯着一个已经走到 0 的钟，什么都不会发生，而那一刻他很可能已经走开了。
   */
  settleTimeout: () => Promise<void>
  /** 赛后深度分析：逐手重算讲解。可以在分析页离开，进度保存在这里。 */
  runDeepAnalysis: () => Promise<void>
  /** 中止深度分析。已经算完的手数保留。 */
  cancelDeep: () => void
}

export const useGameStore = create<GameStore>((set, get) => {
  /** 统一处理一次「引擎调用」：置忙、清错、回填、捕获异常。 */
  async function run<T>(
    action: () => Promise<T>,
    pick: (result: T) => StateDto,
    opts: { coach?: boolean } = {},
  ): Promise<void> {
    set({ busy: true, error: null })
    try {
      const result = await action()
      set({ state: pick(result), selected: null, busy: false })
      // 走子之后异步生成讲解 —— **不 await**。
      // 这是 docs/05 §6.5 的时序：走子立刻完成，讲解晚一点到达，不打断用户。
      if (opts.coach) void get().requestCoach()
    } catch (cause) {
      set({
        busy: false,
        state: await refreshAfterFailure(get().state),
        error: cause instanceof Error ? cause.message : String(cause),
      })
    }
  }

  /**
   * 开一局新的。
   *
   * `fresh` 表示这一局是**从设置页手动开的**（相对于连播里自动接上的下一局）：
   * 手动开局会把连播战绩清零，自动接局则继续累加 —— 不然「连看几十局」记到
   * 第二局就被清了，等于没记。
   */
  async function beginGame(fresh: boolean): Promise<void> {
    set({ nextGameAt: null })
    await run(
      // 限时在这一步下发：开局时把当前档位带给 Rust，由它建钟并开始走秒
      () => bridge.newGame(undefined, timePresetOf(get().timePreset).control),
      (r) => r.state,
    )
    if (get().error !== null) {
      // 开局都没开成（引擎连不上）就别接着排下一局，否则会一直空转
      set({ autoPlaying: false })
      return
    }

    const { mode, autoLevels, autoLevelsChosen } = get()
    set({
      started: true,
      autoPlaying: true,
      notes: {},
      deep: IDLE_DEEP,
      engineInfo: null,
      hints: [],
      hintInfo: null,
      // 没手动选过的一方，每局重摇 —— 连着看几十局才有变化
      autoLevels: mode === 'auto' ? rollAutoLevels(autoLevels, autoLevelsChosen) : autoLevels,
      ...(fresh ? { autoRecord: EMPTY_RECORD } : {}),
    })
    // 新的一局：终局去重和待播队列都要清 —— 手数从 1 重新数，不清的话
    //「第 7 手被将死」在第二局会被当成已经念过，绝杀就不出声了
    resetAnnouncements()
    get().clearCoach()
  }

  return {
    state: null,
    started: false,
    selected: null,
    error: null,
    busy: false,
    flipped: false,
    moveSpeed: loadSpeed(),
    timePreset: loadPreset(),
    voiceEnabled: loadVoiceEnabled(),
    voiceVolume: loadVoiceVolume(),
    voiceSpeed: loadVoiceSpeed(),

    mode: 'hotseat',
    playerColor: 'red',
    difficulty: 'l3',
    // 两边同档起步；只要没手动选过，每开一局都会重摇
    autoLevels: { red: 'l3', black: 'l3' },
    autoLevelsChosen: { red: false, black: false },
    autoPlaying: true,
    nextGameAt: null,
    autoRecord: EMPTY_RECORD,
    thinking: false,
    engineInfo: null,
    hints: [],
    hintInfo: null,
    coachNote: null,
    coachInfo: null,
    coachLoading: false,
    notes: {},
    deep: IDLE_DEEP,

    load: async () => {
      set({ busy: true, error: null })
      // 语音：把设置交给播报模块，顺便把素材烘进浏览器缓存（第一次播报才不卡）
      configure({
        enabled: get().voiceEnabled,
        volume: get().voiceVolume,
        speed: get().voiceSpeed,
      })
      if (get().voiceEnabled) warmUp()
      try {
        set({ state: await bridge.state(), busy: false })
      } catch (cause) {
        set({ busy: false, error: cause instanceof Error ? cause.message : String(cause) })
      }
    },

    clickSquare: (sq) => {
      const { state, selected, mode, playerColor, thinking } = get()
      if (!state || state.status.over || get().busy || thinking) return
      // 复盘期间不能落子。Rust 侧也会拒，但拦在这里能省一次往返，
      // 也不会弹一个「正在复盘」的错误提示吓人。
      if (isReviewing(state)) return
      // 机机对战里人是观众，两边都不该被插手
      if (mode === 'auto') return
      // 人机模式下不允许替引擎走棋
      if (mode === 'engine' && state.side !== playerColor) return

      // 已选中，且点在合法落点上 → 落子
      if (selected !== null) {
        const target = state.legal.find((m) => m.from === selected && m.to === sq)
        if (target) {
          set({ hints: [], hintInfo: null })
          void run(
            () => bridge.move(selected, sq),
            (r) => r.state,
            { coach: true },
          )
          return
        }
      }

      // 点到己方棋子 → 选中（或改选）
      const piece = state.pieces.find((p) => p.sq === sq)
      if (piece && piece.color === state.side) {
        set({ selected: sq, error: null })
        return
      }

      // 其余情况取消选中
      set({ selected: null })
    },

    clearSelection: () => set({ selected: null }),

    playText: async (text) => {
      if (text.trim().length === 0) return
      const { state, mode, playerColor } = get()
      if (!state || isReviewing(state) || mode === 'auto') return
      if (mode === 'engine' && state.side !== playerColor) return
      set({ hints: [], hintInfo: null })
      await run(
        () => bridge.playText(text.trim()),
        (r) => r.state,
        { coach: true },
      )
    },

    playMove: async (from, to) => {
      const { state, mode, playerColor, thinking } = get()
      if (!state || state.status.over || thinking || isReviewing(state)) return
      if (mode === 'auto') return
      if (mode === 'engine' && state.side !== playerColor) return
      set({ hints: [], hintInfo: null })
      await run(
        () => bridge.move(from, to),
        (r) => r.state,
        { coach: true },
      )
    },

    undo: async () => {
      const { mode, state } = get()
      const before = state?.history.length ?? 0
      await run(
        async () => {
          // 一边是引擎时要连退两步，让局面回到「同一方该走」的状态。
          // 只退一步的话，那一方立刻又走回去，看起来像悔棋没生效。
          let result = await bridge.undo()
          if (mode !== 'hotseat' && result.ok) {
            result = await bridge.undo()
          }
          return result
        },
        (r) => r.state,
      )
      // 退掉的那几手的讲解已经作废 —— 留着会让分析页把「已经不作数的棋」
      // 当成对局的一部分列出来。深度分析的进度也要跟着回退，否则重算会跳过它们。
      const after = get().state?.history.length ?? 0
      if (after < before) {
        const kept: Record<number, CoachNote> = {}
        for (const [ply, note] of Object.entries(get().notes)) {
          if (Number(ply) <= after) kept[Number(ply)] = note
        }
        for (const ply of [...deepDone]) {
          if (ply > after) deepDone.delete(ply)
        }
        set({ notes: kept })
      }
      set({ engineInfo: null, hints: [], hintInfo: null })
      get().clearCoach()
    },

    startGame: async () => {
      deepDone.clear()
      await beginGame(true)
    },

    startNextGame: async () => {
      if (startingNext) return
      startingNext = true
      try {
        deepDone.clear()
        await beginGame(false)
      } finally {
        startingNext = false
      }
    },

    armNextGame: () => {
      if (get().mode !== 'auto') return
      set({ nextGameAt: Date.now() + AUTO_GAP_MS })
    },

    finishAutoGame: () => {
      const { state, autoRecord, mode } = get()
      if (mode !== 'auto' || state === null) return
      // 负方是谁 → 另一方赢；没有负方就是和棋
      const loser = state.status.loser
      set({
        autoRecord: {
          red: autoRecord.red + (loser === 'black' ? 1 : 0),
          black: autoRecord.black + (loser === 'red' ? 1 : 0),
          draw: autoRecord.draw + (loser === null ? 1 : 0),
        },
      })
    },

    toggleFlip: () => set((s) => ({ flipped: !s.flipped })),

    setMoveSpeed: (speed) => {
      saveSpeed(speed)
      set({ moveSpeed: speed })
    },

    setTimePreset: (preset) => {
      savePreset(preset)
      set({ timePreset: preset })
    },

    toggleVoice: () => {
      const enabled = !get().voiceEnabled
      const { voiceVolume: volume, voiceSpeed: speed } = get()
      saveVoiceEnabled(enabled)
      configure({ enabled, volume, speed })
      // 打开时才预热：一直关着的话没必要占带宽
      if (enabled) warmUp()
      set({ voiceEnabled: enabled })
    },

    setVoiceVolume: (volume) => {
      const clamped = Math.min(1, Math.max(0, volume))
      saveVoiceVolume(clamped)
      configure({ enabled: get().voiceEnabled, volume: clamped, speed: get().voiceSpeed })
      set({ voiceVolume: clamped })
    },

    setVoiceSpeed: (speed) => {
      const clamped = Math.min(2, Math.max(1, speed))
      saveVoiceSpeed(clamped)
      configure({ enabled: get().voiceEnabled, volume: get().voiceVolume, speed: clamped })
      set({ voiceSpeed: clamped })
    },

    dismissError: () => set({ error: null }),

    setMode: (mode) => {
      const { playerColor, autoLevels, autoLevelsChosen } = get()
      set({
        mode,
        // 人机模式下默认把自己那一侧摆到下方，省得手动翻。
        // 机机对战不翻 —— 红方在下方是看棋的默认朝向。
        flipped: mode === 'engine' && playerColor === 'black',
        engineInfo: null,
        hints: [],
        hintInfo: null,
        // 进机机模式时**先摇一次**，让设置页显示的就是这一局真要用的档位 ——
        // 显示一套、开局用另一套，是最容易被当成 bug 的那种不一致。
        autoLevels: mode === 'auto' ? rollAutoLevels(autoLevels, autoLevelsChosen) : autoLevels,
        // 换模式等于换一轮观战，战绩与倒计时都重来
        ...(mode === 'auto' ? { autoRecord: EMPTY_RECORD, nextGameAt: null } : {}),
      })
    },

    setPlayerColor: (color) => {
      set({
        playerColor: color,
        flipped: color === 'black',
        engineInfo: null,
        hints: [],
        hintInfo: null,
      })
    },

    setDifficulty: (level) => set({ difficulty: level, engineInfo: null, hints: [], hintInfo: null }),

    setAutoLevel: (color, level) =>
      set((s) => ({
        autoLevels: level === null ? s.autoLevels : { ...s.autoLevels, [color]: level },
        // 选「随机」等于把这一方**交还给随机** —— 从此每局重摇
        autoLevelsChosen: { ...s.autoLevelsChosen, [color]: level !== null },
        engineInfo: null,
      })),

    stopAuto: () => {
      // 只清这两个：`autoPlayed` 之类的战绩留着，用户回设置页再开一局时
      // `beginGame(fresh=true)` 会自己清。
      set({ autoPlaying: false, nextGameAt: null })
    },

    toggleAutoPlaying: () => {
      const { autoPlaying, state, mode } = get()
      if (autoPlaying) {
        set({ autoPlaying: false })
        return
      }
      set({ autoPlaying: true })
      // 接着下：还在局中就直接续上；已经终局就**立刻开下一局** ——
      // 按钮上写的也是「立即开始」，说的就是这件事
      if (mode === 'auto' && state?.status.over) void get().startNextGame()
    },

    stepOnce: async () => {
      // 先关连播再走：开着的话这一步走完副作用立刻接着走下一步，
      // 「单步」就等于没按（而且看不出来为什么没用）
      set({ autoPlaying: false })
      await get().enginePlay()
    },

    enginePlay: async () => {
      if (engineInFlight) return
      const { mode, difficulty, autoLevels, state } = get()
      if (!state || state.status.over) return
      // 双人同机里没有引擎的事
      if (mode !== 'engine' && mode !== 'auto') return
      // 复盘时盘面停在过去，别拿历史局面接着往下走
      if (state.cursor !== state.history.length) return

      // 档位按**当前该走的那一方**取 —— 机机对战两边各有一档
      const level = levelForSide(mode, state.side, difficulty, autoLevels)

      engineInFlight = true
      set({ thinking: true, error: null })
      try {
        const response = await bridge.engineMove(level, THINK_MS[level])
        set({
          state: response.state,
          selected: null,
          engineInfo: response.engine.info,
          thinking: false,
          hints: [],
          hintInfo: null,
        })
        void get().requestCoach()
      } catch (cause) {
        set({
          thinking: false,
          // 引擎也可能把自己的表走完（超时判负），同样要重拉一次才知道终局了
          state: await refreshAfterFailure(get().state),
          error: cause instanceof Error ? cause.message : String(cause),
        })
      } finally {
        engineInFlight = false
      }
    },

    requestHint: async () => {
      const { state } = get()
      if (!state || state.status.over) return
      set({ busy: true, error: null })
      try {
        // 提示固定用「大师」档：走棋提示的用途是「告诉我最好的着法」，
        // 若按当前（可能很弱的）难度给建议，建议本身就不够好，反而误导。
        const response = await bridge.hint('l5', THINK_MS.l5, 3)
        set({
          hints: response.hint.suggestions,
          hintInfo: response.hint.info,
          busy: false,
        })
      } catch (cause) {
        set({ busy: false, error: cause instanceof Error ? cause.message : String(cause) })
      }
    },

    clearHint: () => set({ hints: [], hintInfo: null }),

    requestCoach: async () => {
      const { state } = get()
      if (!state || state.history.length === 0) {
        set({ coachNote: null, coachInfo: null, coachLoading: false })
        return
      }
      // 记下这条讲解讲的是第几手。讲解回来时局面可能已经变了，而 `state` 是旧的。
      const ply = state.history.length
      set({ coachLoading: true })
      try {
        // 讲解用的档位比走棋档位高：评价定级的依据是 root_moves 的准确度，
        // 拿弱档位的评分去定级，等级本身就是不准的。
        const response = await bridge.coach('l4', THINK_MS.l4)

        // 攒进「整局讲解」里。这一步**无条件**做：键就是那一手的手数，
        // 回来晚了讲的仍然是那一手，只是没赶上做出场特效而已。
        // 唯一不覆盖的情况是这一手已经被「深度分析」重算过 —— 重算的更准。
        if (!deepDone.has(ply)) {
          set((s) => ({ notes: { ...s.notes, [ply]: response.note } }))
        }

        // 对局页的卡片与出场特效要的是**当前这一步**：期间又走了一步就别回填，
        // 否则会「配着这一步的画面，讲上一步的棋」。
        if (get().state?.history.length === state.history.length) {
          set({ coachNote: response.note, coachInfo: response.info, coachLoading: false })
        } else {
          set({ coachLoading: false })
        }
      } catch (cause) {
        set({
          coachLoading: false,
          error: cause instanceof Error ? cause.message : String(cause),
        })
      }
    },

    clearCoach: () => set({ coachNote: null, coachInfo: null, coachLoading: false }),

    resign: async () => {
      const { state, mode, playerColor } = get()
      if (!state || state.status.over) return
      // 认输方由这里定，不让页面传 —— 这是**对局规则**的一部分，不该散在组件里。
      // 人机对战认输的当然是玩家（哪怕此刻轮到引擎思考）；双人同机则是当前
      // 该走的那一方：谁盯着棋盘，就是谁在认输。
      const loser: Color = mode === 'engine' ? playerColor : state.side
      await run(
        () => bridge.resign(loser),
        (r) => r.state,
      )
      get().clearHint()
    },

    seek: async (ply) => {
      const { state, busy, thinking } = get()
      if (!state || busy || thinking) return
      await run(
        () => bridge.seek(ply),
        (r) => r.state,
      )
    },

    settleTimeout: async () => {
      const { state } = get()
      if (!state || state.status.over) return
      // 两条钟共用一份快照，理论上只有「轮到的那条」会触发；但这个守卫是给
      // 重试与外层重复触发兜底的，避免同一瞬间叠着发几个请求
      if (settlingTimeout) return
      settlingTimeout = true
      try {
        // 前端是**本地插值**走秒的，锚点在「拿到快照那一刻」，所以它归零必然
        // 晚于服务端归零 —— 正常情况下第一次就能判出来。留几次重试是兜住
        // 极端情况（标签页被挂起、定时器被节流），别让玩家永远卡在一个 0 上。
        for (let attempt = 0; attempt < 3; attempt += 1) {
          const response = await bridge.settle()
          set({ state: response.state })
          if (response.state.status.over) return
          // settle 是纯查询，失败会走 catch；走到这里说明服务端说「还没到点」
          await new Promise((resolve) => window.setTimeout(resolve, 400))
        }
      } catch (cause) {
        set({ error: cause instanceof Error ? cause.message : String(cause) })
      } finally {
        settlingTimeout = false
      }
    },

    runDeepAnalysis: async () => {
      const { state, deep } = get()
      if (!state || state.history.length === 0 || deep.running) return
      const total = state.history.length
      // 进度按 `deepDone` 算而不是从 0 起：取消后再点「继续」，进度条接着走
      set({ deep: { running: true, done: deepDone.size, total, error: null } })

      for (let ply = 1; ply <= total; ply += 1) {
        const now = get()
        // 中途可能：点了取消、离开了页面、或者干脆重开了一局
        if (!now.deep.running) return
        if (now.state === null || now.state.history.length !== total) return
        if (deepDone.has(ply)) continue

        try {
          const response = await bridge.analyze(ply, DEEP_LEVEL, DEEP_THINK_MS)
          deepDone.add(ply)
          set((s) => ({
            notes: { ...s.notes, [ply]: response.note },
            deep: { ...s.deep, done: deepDone.size },
          }))
        } catch (cause) {
          // 一次失败就停：连着失败几十次只会刷屏，用户也不知道该看哪条
          set((s) => ({
            deep: {
              ...s.deep,
              running: false,
              error: cause instanceof Error ? cause.message : String(cause),
            },
          }))
          return
        }
      }
      set((s) => ({ deep: { ...s.deep, running: false } }))
    },

    cancelDeep: () => set((s) => ({ deep: { ...s.deep, running: false } })),
  }
})

/** 某格棋子的合法落点。 */
export function legalTargetsFrom(state: StateDto | null, from: string | null): MoveOption[] {
  if (!state || from === null) return []
  return state.legal.filter((m) => m.from === from)
}

/**
 * 盘面是不是停在过去（复盘模式）。
 *
 * 判据只有一条：游标小于记录长度。判断权在 Rust 那边，前端不自己记账 ——
 * 前端要是也维护一份「当前看到第几手」，两边迟早对不上。
 */
export function isReviewing(state: StateDto): boolean {
  return state.cursor < state.history.length
}

/** 现在是否轮到玩家走（人机模式下用来决定要不要触发引擎）。 */
export function isPlayerTurn(state: StateDto | null, mode: GameMode, playerColor: Color): boolean {
  if (!state) return false
  if (mode === 'hotseat') return true
  return state.side === playerColor
}
