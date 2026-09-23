import { useEffect, useRef, useState, type CSSProperties } from 'react'

import type { ClockDto, Color } from '../types'

/**
 * 对局时钟条。
 *
 * # 为什么前端要自己走秒
 *
 * **Rust 是时间的权威**（对齐将来联网对战的服务端权威，见 ADR-014）。
 * 前端拿到的只是「响应生成那一刻的剩余毫秒」，而两次响应之间隔着你思考的时间 ——
 * 直接显示那个数字，秒表就是冻住的。所以这里做**本地插值**：以拿到快照的时刻
 * 为锚点，往后累加，把秒数平滑地走出来；下一次响应到达时重新对表。
 *
 * 前端**不判定超时**。归零后只是变红提示，真正的判负由 Rust 在落子时给出 ——
 * 这样「谁输」只有一个真相源，不会出现前后端各判各的。
 */

const LOW_TIME_MS = 20_000
const DANGER_MS = 10_000

/** 局时：`m:ss`。 */
function fmtBase(ms: number): string {
  const total = Math.max(0, Math.ceil(ms / 1000))
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`
}

/** 本步倒计时：最后 10 秒显示到十分位，让紧迫感看得出来。 */
function fmtStep(ms: number): string {
  const left = Math.max(0, ms)
  return left < 10_000 ? (left / 1000).toFixed(1) : String(Math.ceil(left / 1000))
}

function tone(ms: number): string {
  if (ms <= DANGER_MS) return ' clock__face--danger'
  if (ms <= LOW_TIME_MS) return ' clock__face--low'
  return ''
}

interface FaceProps {
  color: Color
  active: boolean
  baseMs: number
  byoyomi: boolean
  stepLeftMs: number
  stepLimitMs: number
}

function Face({ color, active, baseMs, byoyomi, stepLeftMs, stepLimitMs }: FaceProps) {
  const label = color === 'red' ? '红方' : '黑方'
  // 环上画的是「本步还剩多少」的比例
  const left = stepLimitMs > 0 ? Math.max(0, Math.min(1, stepLeftMs / stepLimitMs)) : 0
  return (
    <div
      className={`clock__face${active ? ' clock__face--active' : ''}${active ? tone(stepLeftMs) : ''}`}
    >
      <span className="clock__label">{label}</span>
      <span className="clock__base">{byoyomi ? '读秒' : fmtBase(baseMs)}</span>
      {active ? (
        <span
          className="clock__ring"
          style={{ '--p': `${(left * 360).toFixed(1)}deg` } as CSSProperties}
        >
          <img src="/hud/timer-ring.svg" alt="" draggable={false} />
          <span className="clock__ring-arc" />
          <span className="clock__ring-num">{fmtStep(stepLeftMs)}</span>
        </span>
      ) : (
        <span className="clock__step clock__step--idle">待机</span>
      )}
    </div>
  )
}

export function ClockBar({
  clock,
  side,
  flipped,
  over,
}: {
  clock: ClockDto | null
  side: Color
  flipped: boolean
  over: boolean
}) {
  const anchor = useRef(0)
  const [elapsed, setElapsed] = useState(0)

  useEffect(() => {
    if (!clock) return
    // 每拿到一份新快照就重新对表，然后本地插值走秒
    anchor.current = Date.now()
    setElapsed(0)
    const timer = window.setInterval(() => setElapsed(Date.now() - anchor.current), 100)
    return () => window.clearInterval(timer)
  }, [clock])

  if (!clock) {
    return (
      <div className="clock clock--off">
        <span className="clock__off">不限时</span>
      </div>
    )
  }

  // 局时只在归属方的钟上走
  const baseOf = (c: Color) => {
    const raw = c === 'red' ? clock.red_ms : clock.black_ms
    return side === c && !over ? Math.max(0, raw - elapsed) : raw
  }
  const stepLeft = over ? clock.step_left_ms : clock.step_left_ms - elapsed

  // 棋盘上方是黑方，所以时钟也按同样的顺序摆，眼睛不用来回找
  const order: Color[] = flipped ? ['red', 'black'] : ['black', 'red']

  return (
    <div className="clock">
      {order.map((c) => (
        <Face
          key={c}
          color={c}
          active={!over && side === c}
          baseMs={baseOf(c)}
          byoyomi={c === 'red' ? clock.red_byoyomi : clock.black_byoyomi}
          stepLeftMs={stepLeft}
          stepLimitMs={clock.step_limit_ms}
        />
      ))}
      <span className="clock__meta">
        局时 {fmtBase(clock.base_secs * 1000)} · 步时 {clock.step_secs}s
        {clock.byoyomi_secs > 0 ? ` · 读秒 ${clock.byoyomi_secs}s` : ''}
      </span>
    </div>
  )
}
