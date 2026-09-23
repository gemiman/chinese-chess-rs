import { useEffect, useRef, useState, type CSSProperties, type MouseEvent } from 'react'

import { flipSquare, iccsToSquare, pointToSquare, squareToIccs, squareToPercent } from '../coords'
import { MOVE_MS, type MoveOption, type MoveSpeed, type StateDto } from '../types'

interface BoardProps {
  state: StateDto
  /** 当前选中的棋子（ICCS），未选中为 `null`。 */
  selected: string | null
  /** 选中棋子的全部合法落点。 */
  legalTargets: MoveOption[]
  /** 是否翻转视角。 */
  flipped: boolean
  /** 是否允许落子。人机模式下轮到引擎时置 false。 */
  interactive: boolean
  /** 走子动画速度档位。 */
  moveSpeed: MoveSpeed
  /** 点击某个交叉点。 */
  onSquare: (sq: string) => void
}

/** 正在飞行的那一步。`id` 取步数，保证连续走子不会复用同一个 DOM 节点。 */
interface Fly {
  id: number
  from: string
  to: string
  sprite: string
  durationMs: number
}

/** 系统是否要求减少动效。 */
function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  )
}

/** 把 ICCS 坐标换算成容器百分比位置（已考虑翻转）。 */
function positionOf(sq: string, flipped: boolean): { x: number; y: number } | null {
  const point = iccsToSquare(sq)
  if (!point) return null
  const display = flipped ? flipSquare(point.col, point.row) : point
  return squareToPercent(display.col, display.row)
}

export function Board({
  state,
  selected,
  legalTargets,
  flipped,
  interactive,
  moveSpeed,
  onSquare,
}: BoardProps) {
  const capturable = new Set(legalTargets.filter((m) => m.capture).map((m) => m.to))
  const reachable = new Set(legalTargets.map((m) => m.to))

  const lastFrom = state.last_move?.from ?? null
  const lastTo = state.last_move?.to ?? null

  const [fly, setFly] = useState<Fly | null>(null)
  const prevSteps = useRef<number | null>(null)
  const steps = state.history.length

  // 走子动画：只在「步数刚好 +1」时播。
  // 悔棋（步数 −1）、重开（归零）、首次载入（prev 仍为 null）都不该有动画。
  useEffect(() => {
    const prev = prevSteps.current
    prevSteps.current = steps
    const last = state.last_move
    if (!last || prev === null || steps !== prev + 1 || prefersReducedMotion()) {
      setFly(null)
      return
    }
    // 走完这步之后，被移动的棋子正落在终点格上 —— 用它当「飞的是哪一枚」
    const moved = state.pieces.find((p) => p.sq === last.to)
    if (!moved) {
      setFly(null)
      return
    }
    setFly({
      id: steps,
      from: last.from,
      to: last.to,
      sprite: moved.sprite,
      durationMs: MOVE_MS[moveSpeed],
    })
  }, [steps, state, moveSpeed])

  // 动画放完就把画面交还给真实棋子。用定时器而不是 onAnimationEnd：
  // 元素中途被替换时那个事件不会触发，会让落点的棋子一直藏着。
  useEffect(() => {
    if (!fly) return
    const timer = window.setTimeout(() => setFly(null), fly.durationMs + 80)
    return () => window.clearTimeout(timer)
  }, [fly])

  // 被将军的那一方的将帅（用于画红色光环）
  const checkedKing = state.in_check
    ? (state.pieces.find((p) => p.kind === 'king' && p.color === state.side)?.sq ?? null)
    : null

  // 飞行棋子的起止位置（跟随翻转）
  const flyFrom = fly ? positionOf(fly.from, flipped) : null
  const flyTo = fly ? positionOf(fly.to, flipped) : null

  function handleClick(event: MouseEvent<HTMLDivElement>) {
    if (!interactive) return
    const rect = event.currentTarget.getBoundingClientRect()
    if (rect.width === 0 || rect.height === 0) return
    const ratioX = (event.clientX - rect.left) / rect.width
    const ratioY = (event.clientY - rect.top) / rect.height
    const displayPoint = pointToSquare(ratioX, ratioY)
    if (!displayPoint) return
    // 显示坐标 → 棋理坐标（翻转是对合，正反同一个函数）
    const chess = flipped ? flipSquare(displayPoint.col, displayPoint.row) : displayPoint
    onSquare(squareToIccs(chess.col, chess.row))
  }

  /** 画一个光效。`fx` 是 `assets/effects/` 下的文件名主干。 */
  function effect(sq: string, fx: string, spin = false) {
    const pos = positionOf(sq, flipped)
    if (!pos) return null
    return (
      <div
        key={`${fx}-${sq}`}
        className={`fx${spin ? ' fx--spin' : ''}`}
        style={{ left: `${pos.x}%`, top: `${pos.y}%` }}
      >
        <img src={`/effects/${fx}.svg`} alt="" draggable={false} />
      </div>
    )
  }

  return (
    <div className="board-wrap">
      <div
        className={`board${interactive ? '' : ' board--locked'}`}
        onClick={handleClick}
        role="group"
        aria-label={`棋盘。当前${state.side === 'red' ? '红' : '黑'}方走子。${
          state.status.over ? state.status.text : `共 ${state.legal.length} 种合法着法`
        }`}
      >
        <img className="board__surface" src="/board/board-classic.svg" alt="" draggable={false} />

        <div className="board__layer">
          {state.pieces.map((piece) => {
            const pos = positionOf(piece.sq, flipped)
            if (!pos) return null
            const classes = ['piece']
            if (piece.color === state.side && !state.status.over && interactive) {
              classes.push('piece--movable')
            }
            if (piece.sq === selected) classes.push('piece--selected')
            if (piece.sq === checkedKing) classes.push('piece--checked')
            // 棋子正在空中飞 → 把终点那枚先藏起来，落地后再由它接管
            if (fly !== null && piece.sq === fly.to) classes.push('piece--hidden')
            return (
              <div
                key={piece.sq}
                className={classes.join(' ')}
                style={{ left: `${pos.x}%`, top: `${pos.y}%` }}
                title={`${piece.color === 'red' ? '红' : '黑'}${piece.glyph} · ${piece.sq}`}
              >
                <img src={`/pieces/${piece.sprite}.svg`} alt="" draggable={false} />
              </div>
            )
          })}
        </div>

        {/* 光效层：**压在棋子之上**。
            素材里的光环半径（27.6/60）正好贴着棋子外沿，放在棋子下面会被整个盖住，
            等于没画；上一着是白弧 + 极淡整格暖色，压在上面才看得出来是个「弧」。 */}
        <div className="board__layer board__layer--fx">
          {[...reachable].map((sq) => effect(sq, capturable.has(sq) ? 'fx-capture' : 'fx-move'))}
          {selected !== null ? effect(selected, 'fx-select') : null}
          {lastFrom !== null ? effect(lastFrom, 'fx-last-move', true) : null}
          {lastTo !== null ? effect(lastTo, 'fx-last-move', true) : null}
        </div>

        {/* 飞行棋子：从起点直线滑到终点。外层做成与棋盘同尺寸，
            这样 keyframes 里 translate 的百分比就正好是棋盘百分比。 */}
        {fly !== null && flyFrom !== null && flyTo !== null ? (
          <div
            key={fly.id}
            className="fly"
            style={
              {
                '--fly-dx': `${flyFrom.x - flyTo.x}%`,
                '--fly-dy': `${flyFrom.y - flyTo.y}%`,
                '--fly-ms': `${fly.durationMs}ms`,
              } as CSSProperties
            }
          >
            <div className="fly__piece" style={{ left: `${flyTo.x}%`, top: `${flyTo.y}%` }}>
              <img src={`/pieces/${fly.sprite}.svg`} alt="" draggable={false} />
            </div>
          </div>
        ) : null}
      </div>
    </div>
  )
}
