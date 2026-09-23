import type { MouseEvent } from 'react'

import { flipSquare, iccsToSquare, pointToSquare, squareToIccs, squareToPercent } from '../coords'
import type { MoveOption, StateDto } from '../types'

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
  /** 点击某个交叉点。 */
  onSquare: (sq: string) => void
}

/** 把 ICCS 坐标换算成容器百分比位置（已考虑翻转）。 */
function positionOf(sq: string, flipped: boolean): { x: number; y: number } | null {
  const point = iccsToSquare(sq)
  if (!point) return null
  const display = flipped ? flipSquare(point.col, point.row) : point
  return squareToPercent(display.col, display.row)
}

export function Board({ state, selected, legalTargets, flipped, interactive, onSquare }: BoardProps) {
  const capturable = new Set(legalTargets.filter((m) => m.capture).map((m) => m.to))
  const reachable = new Set(legalTargets.map((m) => m.to))

  const lastFrom = state.last_move?.from ?? null
  const lastTo = state.last_move?.to ?? null

  // 被将军的那一方的将帅（用于画红色光环）
  const checkedKing = state.in_check
    ? (state.pieces.find((p) => p.kind === 'king' && p.color === state.side)?.sq ?? null)
    : null

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

  /** 画一个标记。 */
  function marker(sq: string, className: string) {
    const pos = positionOf(sq, flipped)
    if (!pos) return null
    return (
      <span
        key={`${className}-${sq}`}
        className={`marker ${className}`}
        style={{ left: `${pos.x}%`, top: `${pos.y}%` }}
      />
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

        {/* 标记层在棋子之下：圆环大于棋子，故仍会露在棋子外圈 */}
        <div className="board__layer">
          {[...reachable].map((sq) =>
            capturable.has(sq) ? marker(sq, 'marker--ring') : marker(sq, 'marker--dot'),
          )}

          {selected !== null ? marker(selected, 'marker--selected') : null}
        </div>

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

        {/* 上一着标记单独一层，压在棋子**之上** ——
            它标的是「落点」那一格，而落点上通常正好有棋子；若放在棋子之下，
            方框比棋子小，会被完全盖住，等于没画。 */}
        <div className="board__layer board__layer--overlay">
          {lastFrom ? marker(lastFrom, 'marker--last') : null}
          {lastTo ? marker(lastTo, 'marker--last') : null}
        </div>
      </div>
    </div>
  )
}
