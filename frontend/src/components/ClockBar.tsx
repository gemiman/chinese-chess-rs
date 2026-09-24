import { useEffect, useRef, useState, type CSSProperties } from 'react'

import { DIFFICULTIES, type ClockDto, type Color, type DifficultyId } from '../types'

/**
 * 对局时钟条。放在棋盘**上方**显示对手、**下方**显示自己 ——
 * 与参考图的结构一致：眼睛不用在同一个地方找两个人的时间。
 *
 * # 一个人一个「圆盘」
 *
 * 圆盘是**头像位**（现在只放步时倒计时，将来换成真实头像 + 会员角标），
 * 步时倒计时环套在圆盘外面，局时挂在圆盘下面。
 *
 * ⚠️ 环的底板素材 `timer-ring.svg` 里有一块**深色实心圆**（r=45/70）。
 * 这是给「环里只有数字」的用法准备的；这里环里要放头像，所以刻意**不用那张底板**，
 * 只把进度弧画出来（`conic-gradient` + 径向遮罩），否则头像会被整块盖住。
 *
 * # 为什么前端要自己走秒
 *
 * **Rust 是时间的权威**（对齐联网对战的服务端权威，见 ADR-014）。
 * 前端拿到的只是「响应生成那一刻的剩余毫秒」，两次响应之间隔着你思考的时间 ——
 * 直接显示那个数字，秒表就是冻住的。所以这里做**本地插值**：以拿到快照的时刻
 * 为锚点往后累加，下一次响应到达时重新对表。
 *
 * 前端**不判定超时**。归零后只是变红提示，真正的判负由 Rust 在落子时给出。
 */

const LOW_TIME_MS = 20_000
const DANGER_MS = 10_000

/** 局时：`m:ss`。 */
function fmtBase(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000))
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
}

/** 步时倒计时：最后 10 秒显示到十分位，让紧迫感看得出来。 */
function fmtStep(ms: number): string {
  const left = Math.max(0, ms)
  return left < 10_000 ? (left / 1000).toFixed(1) : String(Math.ceil(left / 1000))
}

function tone(ms: number): string {
  if (ms <= DANGER_MS) return ' clock--danger'
  if (ms <= LOW_TIME_MS) return ' clock--low'
  return ''
}

/** 把「上次拿到快照到现在」过了多久走起来。 */
function useElapsed(clock: ClockDto | null): number {
  const anchor = useRef(0)
  const [elapsed, setElapsed] = useState(0)
  useEffect(() => {
    if (!clock) return
    anchor.current = Date.now()
    setElapsed(0)
    const timer = window.setInterval(() => setElapsed(Date.now() - anchor.current), 100)
    return () => window.clearInterval(timer)
  }, [clock])
  return elapsed
}

interface ClockBarProps {
  clock: ClockDto | null
  /** 当前走子方。 */
  side: Color
  flipped: boolean
  over: boolean
  /**
   * 盘面是不是停在过去（复盘模式）。
   *
   * 复盘时那条「局时」是**终局的残余时间**，拿它配第 10 手的局面是错的，
   * 所以改成一个「复盘」标记 —— 说不知道，好过说错的。
   */
  reviewing: boolean
  /** 放在棋盘上方还是下方 —— 决定这一条显示哪一方。 */
  position: 'top' | 'bottom'
  /**
   * 本方步时归零时通知外面（用来**当场**结算超时）。
   *
   * 只有**轮到走子**的那一条钟会调它，而且每条钟对同一份快照只调一次。
   */
  onStepExpired?: () => void
  /**
   * 机机对战时传进来：在侧名旁边标出这一方是哪个档位的 AI。
   *
   * 让 ClockBar 自己按 `shown` 取档位，而不是由外面算好颜色再传进来 ——
   * 「上方是哪一方」的翻转规则只在这一个文件里，传颜色就等于把它复制到外面。
   */
  autoLevels?: Record<Color, DifficultyId>
  /** 机机对战：这一方已经赢了几局（连播的累计战绩）。 */
  autoRecord?: Record<Color, number>
}

export function ClockBar({
  clock,
  side,
  flipped,
  over,
  reviewing,
  position,
  onStepExpired,
  autoLevels,
  autoRecord,
}: ClockBarProps) {
  const elapsed = useElapsed(clock)

  // 上方永远是「棋盘对面那一方」，与翻转保持一致
  const shown: Color = position === 'top' ? (flipped ? 'red' : 'black') : flipped ? 'black' : 'red'
  const label = shown === 'red' ? '红方' : '黑方'
  const levelLabel =
    autoLevels === undefined
      ? null
      : (DIFFICULTIES.find((d) => d.id === autoLevels[shown])?.label ?? null)
  const wins = autoRecord === undefined ? null : autoRecord[shown]

  // ⚠️ `active` 与 `stepLeftMs` 必须在下面那个「不限时」提前 return **之前**算出来 ——
  // Hook 不能写在条件分支后面。不限时时步时用无穷大表示「永远不会归零」。
  const active = clock !== null && !over && !reviewing && side === shown
  const stepLeftMs =
    clock === null
      ? Number.POSITIVE_INFINITY
      : over
        ? clock.step_left_ms
        : clock.step_left_ms - elapsed

  // 步时归零 → 通知外面当场结算超时。
  //
  // 以前超时**只有在有人试着走棋时**才会被发现：玩家盯着一个已经走到 0 的钟，
  // 什么都不会发生，非得再点一下棋盘才被判负 —— 而那一刻他很可能已经走开了。
  // 这里用 `reported` 记住「这一份快照已经报过了」：不然每 100 毫秒的走秒
  // 都会报一次，一秒能发出十个结算请求。
  const reported = useRef<ClockDto | null>(null)
  useEffect(() => {
    if (!active || clock === null || onStepExpired === undefined) return
    if (stepLeftMs > 0) return
    if (reported.current === clock) return
    reported.current = clock
    onStepExpired()
  }, [active, stepLeftMs, clock, onStepExpired])

  if (!clock) {
    // 不限时也用同一套三列结构，只是圆盘里没有倒计时。
    // 复盘样式（虚线、淡出）同样要带上 —— 不限时的钟没有「读数会误导」的问题，
    // 但两条钟在复盘时长得不一样会更让人困惑。
    return (
      <div className={`clock clock--${position} clock--off${reviewing ? ' clock--reviewing' : ''}`}>
        <span className="clock__side clock__side--left">
          {wins !== null ? <span className="clock__record">{wins} 胜</span> : null}
          {reviewing ? <span className="clock__config">复盘</span> : null}
        </span>
        <span className="clock__avatar">
          <span className="clock__dial">
            <span className="clock__disc">
              <img
                className="clock__disc-avatar"
                src="/hud/avatar-placeholder.svg"
                alt=""
                draggable={false}
              />
            </span>
          </span>
          <span className="clock__base">{reviewing ? '—' : '不限时'}</span>
        </span>
        <span className="clock__side clock__side--right">
          <span className="clock__label">
            {label}
            {levelLabel !== null ? <span className="clock__level">{levelLabel}</span> : null}
          </span>
        </span>
      </div>
    )
  }

  const byoyomi = shown === 'red' ? clock.red_byoyomi : clock.black_byoyomi

  // 局时只在归属方的钟上走（`active` 与 `stepLeftMs` 在上面算好了）
  const raw = shown === 'red' ? clock.red_ms : clock.black_ms
  const baseMs = active ? Math.max(0, raw - elapsed) : raw
  const left = clock.step_limit_ms > 0 ? Math.max(0, Math.min(1, stepLeftMs / clock.step_limit_ms)) : 0

  return (
    <div
      className={`clock clock--${position}${active ? ' clock--active' : ''}${
        active ? tone(stepLeftMs) : ''
      }${reviewing ? ' clock--reviewing' : ''}`}
    >
      {/* 三列网格：左右各 1fr、中间 auto —— 圆盘才真正落在正中，
          不会被右边的文字顶偏。文字分列两侧，中间留白给头像。 */}
      <span className="clock__side clock__side--left">
        {wins !== null ? <span className="clock__record">{wins} 胜</span> : null}
        <span className="clock__config">
          {reviewing ? (
            '复盘'
          ) : (
            <>
              局时 {fmtBase(clock.base_secs * 1000)} · 步时 {clock.step_secs}s
              {clock.byoyomi_secs > 0 ? ` · 读秒 ${clock.byoyomi_secs}s` : ''}
            </>
          )}
        </span>
      </span>

      {/* 圆盘 = 头像位。步时倒计时环套在外面，局时挂在下面。
          **轮到这一方**时圆盘显示步时倒计时，**没轮到**时显示头像。 */}
      <span className="clock__avatar">
        <span className="clock__dial">
          <span className="clock__disc">
            {active ? (
              <span className="clock__disc-num">{fmtStep(stepLeftMs)}</span>
            ) : (
              <img
                className="clock__disc-avatar"
                src="/hud/avatar-placeholder.svg"
                alt=""
                draggable={false}
              />
            )}
          </span>
          <span
            className="clock__ring-arc"
            style={{ '--p': `${(left * 360).toFixed(1)}deg` } as CSSProperties}
          />
        </span>
        {/* 复盘时那一格不显示局时：它是终局的残余时间，配不上当时的盘面 */}
        <span className="clock__base">
          {reviewing ? '—' : byoyomi ? '读秒' : fmtBase(baseMs)}
        </span>
      </span>

      <span className="clock__side clock__side--right">
        <span className="clock__label">
          {label}
          {levelLabel !== null ? <span className="clock__level">{levelLabel}</span> : null}
          {active ? <span className="clock__turn">走子中</span> : null}
        </span>
      </span>
    </div>
  )
}
