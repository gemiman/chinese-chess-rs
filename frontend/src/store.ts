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
import {
  THINK_MS,
  type CoachNote,
  type Color,
  type DifficultyId,
  type EngineInfo,
  type GameMode,
  type HintItem,
  type MoveOption,
  type StateDto,
} from './types'

/**
 * 「引擎正在思考」的同步守卫。
 *
 * 为什么不只用 store 里的 `thinking` 字段：React 18 的 StrictMode 会把副作用
 * 跑两遍，而 `set()` 之后组件重渲染需要一次微任务 —— 两次调用之间 `thinking`
 * 还是 `false`，于是会发出**两次**引擎请求，走出两步棋。
 * 用一个模块级变量做同步短路最稳。
 */
let engineInFlight = false

interface GameStore {
  state: StateDto | null
  /** 当前选中的棋子（ICCS 坐标）；未选中为 `null`。 */
  selected: string | null
  error: string | null
  /** 是否正在等待引擎响应。 */
  busy: boolean
  /** 是否翻转视角（执黑时用）。 */
  flipped: boolean

  /** 对局模式：双人热座 / 人机对战。 */
  mode: GameMode
  /** 人机模式下玩家执哪一方。 */
  playerColor: Color
  /** 人机模式下的引擎档位。 */
  difficulty: DifficultyId
  /** 引擎是否正在思考。 */
  thinking: boolean
  /** 最近一次引擎搜索的元信息。 */
  engineInfo: EngineInfo | null
  /** 走棋提示（按需请求）。 */
  hints: HintItem[]
  hintInfo: EngineInfo | null

  /** 最后一步的战法讲解。 */
  coachNote: CoachNote | null
  coachInfo: EngineInfo | null
  /** 讲解生成中（比走子慢，因为它要跑一次搜索）。 */
  coachLoading: boolean

  /** 拉取一次局面。 */
  load: () => Promise<void>
  /** 点击某个交叉点。选中 / 落子 / 取消 三态由这里统一决定。 */
  clickSquare: (sq: string) => void
  /** 清空选中。 */
  clearSelection: () => void
  /** 按记谱或坐标走一步（供侧栏输入框使用）。 */
  playText: (text: string) => Promise<void>
  /** 直接走一步（供推荐着法条使用）。 */
  playMove: (from: string, to: string) => Promise<void>
  /** 悔一步。人机模式下会连退两步，回到玩家自己的回合。 */
  undo: () => Promise<void>
  /** 重开。 */
  reset: () => Promise<void>
  /** 切换视角。 */
  toggleFlip: () => void
  /** 手动清除错误提示。 */
  dismissError: () => void

  setMode: (mode: GameMode) => void
  setPlayerColor: (color: Color) => void
  setDifficulty: (level: DifficultyId) => void
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
        error: cause instanceof Error ? cause.message : String(cause),
      })
    }
  }

  return {
    state: null,
    selected: null,
    error: null,
    busy: false,
    flipped: false,

    mode: 'hotseat',
    playerColor: 'red',
    difficulty: 'l3',
    thinking: false,
    engineInfo: null,
    hints: [],
    hintInfo: null,
    coachNote: null,
    coachInfo: null,
    coachLoading: false,

    load: async () => {
      set({ busy: true, error: null })
      try {
        set({ state: await bridge.state(), busy: false })
      } catch (cause) {
        set({ busy: false, error: cause instanceof Error ? cause.message : String(cause) })
      }
    },

    clickSquare: (sq) => {
      const { state, selected, mode, playerColor, thinking } = get()
      if (!state || state.status.over || get().busy || thinking) return
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
      if (mode === 'engine' && state && state.side !== playerColor) return
      set({ hints: [], hintInfo: null })
      await run(
        () => bridge.playText(text.trim()),
        (r) => r.state,
        { coach: true },
      )
    },

    playMove: async (from, to) => {
      const { state, mode, playerColor, thinking } = get()
      if (!state || state.status.over || thinking) return
      if (mode === 'engine' && state.side !== playerColor) return
      set({ hints: [], hintInfo: null })
      await run(
        () => bridge.move(from, to),
        (r) => r.state,
        { coach: true },
      )
    },

    undo: async () => {
      const { mode } = get()
      await run(
        async () => {
          // 人机模式：连退两步，让局面回到「玩家该走」的状态。
          // 只退一步的话，玩家一悔棋引擎立刻又走回去，看起来像悔棋没生效。
          let result = await bridge.undo()
          if (mode === 'engine' && result.ok) {
            result = await bridge.undo()
          }
          return result
        },
        (r) => r.state,
      )
      set({ engineInfo: null, hints: [], hintInfo: null })
      get().clearCoach()
    },

    reset: async () => {
      await run(
        () => bridge.newGame(),
        (r) => r.state,
      )
      set({ engineInfo: null, hints: [], hintInfo: null })
      get().clearCoach()
    },

    toggleFlip: () => set((s) => ({ flipped: !s.flipped })),

    dismissError: () => set({ error: null }),

    setMode: (mode) => {
      const { playerColor } = get()
      set({
        mode,
        // 人机模式下默认把自己那一侧摆到下方，省得手动翻
        flipped: mode === 'engine' && playerColor === 'black',
        engineInfo: null,
        hints: [],
        hintInfo: null,
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

    enginePlay: async () => {
      if (engineInFlight) return
      const { mode, difficulty, state } = get()
      if (mode !== 'engine' || !state || state.status.over) return

      engineInFlight = true
      set({ thinking: true, error: null })
      try {
        const response = await bridge.engineMove(difficulty, THINK_MS[difficulty])
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
      set({ coachLoading: true })
      try {
        // 讲解用的档位比走棋档位高：评价定级的依据是 root_moves 的准确度，
        // 拿弱档位的评分去定级，等级本身就是不准的。
        const response = await bridge.coach('l4', THINK_MS.l4)
        // 期间可能又走了一步 —— 只在步数仍匹配时回填，避免讲错步
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
  }
})

/** 某格棋子的合法落点。 */
export function legalTargetsFrom(state: StateDto | null, from: string | null): MoveOption[] {
  if (!state || from === null) return []
  return state.legal.filter((m) => m.from === from)
}

/** 现在是否轮到玩家走（人机模式下用来决定要不要触发引擎）。 */
export function isPlayerTurn(state: StateDto | null, mode: GameMode, playerColor: Color): boolean {
  if (!state) return false
  if (mode === 'hotseat') return true
  return state.side === playerColor
}
