/**
 * 与桥接服务（`xq-bridge`）之间的数据契约。
 *
 * 字段名刻意与 Rust 侧 `serde` 的默认 snake_case 输出保持一致，
 * 不做前端映射 —— 少一层转换就少一处不一致的可能。
 */

export type Color = 'red' | 'black'

/**
 * 局面状态种类。
 *
 * `timeout` 与 `resign` 是**会话层**给出的终局 —— 棋盘上没有任何痕迹能证明
 * 「谁的表走完了」或「谁认输了」，所以 `xq-core::GameStatus` 里没有它们。
 */
export type StatusKind =
  | 'ongoing'
  | 'check'
  | 'checkmate'
  | 'stalemate'
  | 'draw'
  | 'timeout'
  | 'resign'

/** 局面状态。 */
export interface StatusDto {
  kind: StatusKind
  /** 面向用户的中文说明。 */
  text: string
  over: boolean
  /** 将死 / 困毙时的负方；其余为 `null`。 */
  loser: Color | null
}

/** 棋盘上的一枚棋子。 */
export interface PieceDto {
  /** ICCS 坐标，如 `h2`。 */
  sq: string
  /** 列 0..8（0 = a 列）。 */
  col: number
  /** 行 0..9（0 = 红方底线）。 */
  row: number
  color: Color
  /** king / advisor / elephant / horse / chariot / cannon / pawn */
  kind: string
  /** `assets/pieces/<sprite>.svg` 的文件名主干。 */
  sprite: string
  /** 中文棋子字，用于无障碍朗读。 */
  glyph: string
}

/** 当前局面下可选的一步。 */
export interface MoveOption {
  from: string
  to: string
  /** 紧凑串，如 `h2e2`。 */
  iccs: string
  /** 中文记谱，如 `炮二平五`。 */
  notation: string
  capture: boolean
}

/** 已经走过的一步。 */
export interface PlayedMove extends MoveOption {
  /** check / capture / escape / interpose / capture_attacker / idle */
  nature: string
  /** 性质的中文说明。 */
  nature_text: string
  side: Color
}

/** 完整局面快照。前端渲染所需的一切都在这里。 */
export interface StateDto {
  fen: string
  side: Color
  in_check: boolean
  status: StatusDto
  pieces: PieceDto[]
  legal: MoveOption[]
  last_move: PlayedMove | null
  history: PlayedMove[]
  /**
   * 复盘游标：盘面停在「第几手走完」之后（`0` = 开局）。
   *
   * 等于 `history.length` 时看的就是最新局面；小于它说明**正在复盘**。
   * 复盘期间不能落子、不能悔棋 —— 否则会把这一局的记录截断在半路上。
   */
  cursor: number
  halfmove_clock: number
  fullmove_number: number
  /** 棋钟。`null` 表示这一局不限时。 */
  clock: ClockDto | null
}

export interface MoveResponse {
  ok: boolean
  played: PlayedMove
  state: StateDto
}

export interface StateResponse {
  ok: boolean
  state: StateDto
}

// ---------------------------------------------------------------- 引擎

/** 引擎搜索的元信息。 */
export interface EngineInfo {
  /** `l1` ~ `l5` */
  level: string
  /** 中文档位名 */
  level_label: string
  depth: number
  /** 厘兵，走子方视角 */
  score: number
  nodes: number
  /** 杀棋距离；非杀棋为 null */
  mate_in: number | null
  /** 是否因时间用尽被中断 */
  stopped: boolean
  think_ms: number
}

export interface EngineMoveResponse {
  ok: boolean
  engine: { played: PlayedMove; info: EngineInfo }
  state: StateDto
}

/** 一条推荐着法。 */
export interface HintItem {
  from: string
  to: string
  iccs: string
  notation: string
  score: number
  capture: boolean
}

export interface HintResponse {
  ok: boolean
  hint: { suggestions: HintItem[]; info: EngineInfo }
}

// ---------------------------------------------------------------- 战法讲解

/** 评价等级。与 `xq-coach` 的 `MoveLevel` 一一对应。 */
export type MoveLevel = 'best' | 'good' | 'dubious' | 'blunder' | 'missed'

/** 战术类别，按**可判定性**分层。 */
export type TacticCategory = 'structure' | 'relation' | 'formation' | 'opening'

export interface TacticTag {
  id: string
  name: string
  category: TacticCategory
  /** 0.0 ~ 1.0。棋形层低于 0.7 时 UI 应以弱样式展示。 */
  confidence: number
}

export interface CoachNote {
  ply: number
  side: Color
  mv_iccs: string
  notation: string

  level: MoveLevel
  /** 分差（厘兵）。 */
  score_loss: number
  score_before: number
  score_after: number

  tactics: TacticTag[]

  headline: string
  detail: string
  suggestion: string | null

  source: 'local' | 'llm'
  llm_status: 'none' | 'pending' | 'enhanced' | 'failed'

  pv: string[]
  opening_name: string | null
  template_id: string
  /** 走了兜底模板 —— 兜底不计入覆盖率。 */
  used_fallback: boolean
}

export interface CoachResponse {
  ok: boolean
  note: CoachNote
  info: EngineInfo
}

// ---------------------------------------------------------------- 对局模式

export type GameMode = 'hotseat' | 'engine'

export type DifficultyId = 'l1' | 'l2' | 'l3' | 'l4' | 'l5'

/** 难度档位表。文案与 `xq-ai` 保持一致 —— 改这里时记得同步 Rust 侧。 */
export const DIFFICULTIES: { id: DifficultyId; label: string; subtitle: string }[] = [
  { id: 'l1', label: '入门', subtitle: '刚学会走法，适合熟悉规则' },
  { id: 'l2', label: '初级', subtitle: '会基本的吃子与防守' },
  { id: 'l3', label: '中级', subtitle: '有基本战术意识，会计算几步' },
  { id: 'l4', label: '高级', subtitle: '会布局与组合战术，算得较深' },
  { id: 'l5', label: '大师', subtitle: '搜索更深，思考更久' },
]

/** 各档位的思考时间（毫秒）。L4/L5 给足时间，弱档位要快。 */
export const THINK_MS: Record<DifficultyId, number> = {
  l1: 300,
  l2: 600,
  l3: 1_200,
  l4: 2_000,
  l5: 3_000,
}

/**
 * 赛后「深度分析」用的档位与每手思考时间。
 *
 * 档位取「高级」而不是当前的走棋档位：评价定级的依据是 `root_moves` 的准确度，
 * 拿弱档位的评分去定级，等级本身就是不准的（与走棋后的即时讲解同一个理由）。
 *
 * 时间是**折中**：即时讲解每手给 2000 毫秒无所谓（一局只算最后一步），
 * 但整局重算要算几十手 —— 2000 毫秒 × 60 手 = 两分钟，没人等得下去。
 */
export const DEEP_LEVEL: DifficultyId = 'l4'
export const DEEP_THINK_MS = 800

// ---------------------------------------------------------------- 走子动画

/** 走子动画速度档位。 */
export type MoveSpeed = 'fast' | 'normal' | 'slow'

/** 各档位的动画时长（毫秒）。「慢动作」档是给复盘看棋用的。 */
export const MOVE_MS: Record<MoveSpeed, number> = {
  fast: 180,
  normal: 400,
  slow: 1_000,
}

/** 速度档位表，用于侧栏渲染。 */
export const MOVE_SPEEDS: { id: MoveSpeed; label: string }[] = [
  { id: 'fast', label: '快' },
  { id: 'normal', label: '正常' },
  { id: 'slow', label: '慢动作' },
]

// ---------------------------------------------------------------- 限时

/**
 * 棋钟快照，来自 Rust。
 *
 * **Rust 是时间的权威**（对齐将来的服务端权威，见 ADR-014）；前端只在两次
 * 响应之间做本地插值，把秒数平滑地显示出来，绝不自己判定超时。
 */
export interface ClockDto {
  /** 局时（秒） */
  base_secs: number
  /** 步时（秒） */
  step_secs: number
  /** 读秒（秒） */
  byoyomi_secs: number
  /** 红方局时剩余（毫秒）；已进读秒则为 0 */
  red_ms: number
  black_ms: number
  red_byoyomi: boolean
  black_byoyomi: boolean
  /** 当前走子方本步剩余（毫秒），可能为负 */
  step_left_ms: number
  /** 当前走子方本步的上限（毫秒），即超时判负的阈值。由 Rust 给出，
      前端不自己推 min(步时, 局时剩余)，免得同一套规则两边各写一遍。 */
  step_limit_ms: number
}

/** 限时配置（发给 Rust）。`null` 表示不限时。 */
export interface TimeControlInput {
  base_secs: number
  step_secs: number
  byoyomi_secs: number
}

export type TimePresetId = 'none' | 'blitz' | 'standard' | 'slow'

/**
 * 四档时间预设。
 *
 * 「读秒」取与步时相同的秒数 —— 这是最常见的配置：局时耗尽后，每步的
 * 思考上限和之前一样，只是不再有总时间可花。
 */
export const TIME_PRESETS: {
  id: TimePresetId
  label: string
  hint: string
  control: TimeControlInput | null
}[] = [
  { id: 'none', label: '不限时', hint: '随便想，不催', control: null },
  {
    id: 'blitz',
    label: '快棋',
    hint: '局时 5 分 · 步时 20 秒',
    control: { base_secs: 300, step_secs: 20, byoyomi_secs: 20 },
  },
  {
    id: 'standard',
    label: '标准',
    hint: '局时 10 分 · 步时 30 秒',
    control: { base_secs: 600, step_secs: 30, byoyomi_secs: 30 },
  },
  {
    id: 'slow',
    label: '慢棋',
    hint: '局时 20 分 · 步时 60 秒',
    control: { base_secs: 1200, step_secs: 60, byoyomi_secs: 60 },
  },
]

export function timePresetOf(id: TimePresetId) {
  return TIME_PRESETS.find((p) => p.id === id) ?? TIME_PRESETS[0]
}
