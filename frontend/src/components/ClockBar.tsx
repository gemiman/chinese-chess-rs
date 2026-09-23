import { useEffect, useRef, useState, type CSSProperties } from 'react'

import type { ClockDto, Color } from '../types'

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
  /** 放在棋盘上方还是下方 —— 决定这一条显示哪一方。 */
  position: 'top' | 'bottom'
}

export function ClockBar({ clock, side, flipped, over, position }: ClockBarProps) {
  const elapsed = useElapsed(clock)

  // 上方永远是「棋盘对面那一方」，与翻转保持一致
  const shown: Color = position === 'top' ? (flipped ? 'red' : 'black') : flipped ? 'black' : 'red'
  const label = shown === 'red' ? '红方' : '黑方'

  if (!clock) {
    // 不限时也用同一套三列结构，只是圆盘里没有倒计时
    return (
      <div className={`clock clock--${position} clock--off`}>
        <span className="clock__side clock__side--left" />
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
          <span className="clock__base">不限时</span>
        </span>
        <span className="clock__side clock__side--right">
          <span className="clock__label">{label}</span>
        </span>
      </div>
    )
  }

  const active = !over && side === shown
  const byoyomi = shown === 'red' ? clock.red_byoyomi : clock.black_byoyomi

  // 局时只在归属方的钟上走
  const raw = shown === 'red' ? clock.red_ms : clock.black_ms
  const baseMs = active ? Math.max(0, raw - elapsed) : raw
  const stepLeftMs = over ? clock.step_left_ms : clock.step_left_ms - elapsed
  const left = clock.step_limit_ms > 0 ? Math.max(0, Math.min(1, stepLeftMs / clock.step_limit_ms)) : 0

  return (
    <div
      className={`clock clock--${position}${active ? ' clock--active' : ''}${
        active ? tone(stepLeftMs) : ''
      }`}
    >
      {/* 三列网格：左右各 1fr、中间 auto —— 圆盘才真正落在正中，
          不会被右边的文字顶偏。文字分列两侧，中间留白给头像。 */}
      <span className="clock__side clock__side--left">
        <span className="clock__config">
          局时 {fmtBase(clock.base_secs * 1000)} · 步时 {clock.step_secs}s
          {clock.byoyomi_secs > 0 ? ` · 读秒 ${clock.byoyomi_secs}s` : ''}
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
        <span className="clock__base">{byoyomi ? '读秒' : fmtBase(baseMs)}</span>
      </span>

      <span className="clock__side clock__side--right">
        <span className="clock__label">
          {label}
          {active ? <span className="clock__turn">走子中</span> : null}
        </span>
      </span>
    </div>
  )
}
