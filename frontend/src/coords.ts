/**
 * 棋盘坐标系换算。
 *
 * # 三套坐标
 *
 * | 名称 | 含义 |
 * |---|---|
 * | 棋理坐标 `(col, row)` | `col` 0..8 = a..i 列；`row` 0..9，**`row 0` 是红方底线** |
 * | 视图坐标 | `assets/board/board-classic.svg` 的像素空间，viewBox `0 0 560 620` |
 * | 显示比例 | 相对棋盘容器的 `0..1`，用于把棋子摆成绝对定位 |
 *
 * # ⚠️ 必须垂直翻转
 *
 * SVG 与屏幕的 y 轴**向下**，而棋理的 `row 0`（红方底线）在视觉**下方**。
 * 因此视图 y = `MARGIN + (ROWS - 1 - row) * CELL`。
 * 忘了这一步会导致红黑上下颠倒 —— 这是本项目已经踩过一次的坑
 * （见 `scripts/build_prototype.py` 的修复记录）。
 */

export const VIEW_W = 560
export const VIEW_H = 620
export const MARGIN = 40
export const CELL = 60
export const COLS = 9
export const ROWS = 10

export interface Point {
  col: number
  row: number
}

/** 棋理坐标 → 视图像素坐标（交叉点中心）。 */
export function squareToView(col: number, row: number): { x: number; y: number } {
  return {
    x: MARGIN + col * CELL,
    y: MARGIN + (ROWS - 1 - row) * CELL,
  }
}

/** 棋理坐标 → 容器内的百分比坐标（0..100），可直接喂给 CSS。 */
export function squareToPercent(col: number, row: number): { x: number; y: number } {
  const { x, y } = squareToView(col, row)
  return { x: (x / VIEW_W) * 100, y: (y / VIEW_H) * 100 }
}

/**
 * 容器内的比例坐标（0..1）→ 棋理坐标。
 *
 * 只有当点击位置足够接近某个交叉点（默认容差 0.45 格）时才返回结果，
 * 否则返回 `null` —— 这样才能把「点到格子中间的空隙」与「点到交叉点」区分开，
 * 避免误走。
 */
export function pointToSquare(xRatio: number, yRatio: number, tolerance = 0.45): Point | null {
  const vx = xRatio * VIEW_W
  const vy = yRatio * VIEW_H

  const colF = (vx - MARGIN) / CELL
  const displayRowF = (vy - MARGIN) / CELL

  const col = Math.round(colF)
  const displayRow = Math.round(displayRowF)

  if (col < 0 || col >= COLS) return null
  if (displayRow < 0 || displayRow >= ROWS) return null
  if (Math.abs(colF - col) > tolerance) return null
  if (Math.abs(displayRowF - displayRow) > tolerance) return null

  // 显示行 → 棋理行
  return { col, row: ROWS - 1 - displayRow }
}

/**
 * 翻转视角。用于「执黑时棋盘旋转 180°」。
 *
 * 这是一个**对合**（连续调用两次回到原值），所以正反两个方向共用同一个函数。
 */
export function flipSquare(col: number, row: number): Point {
  return { col: COLS - 1 - col, row: ROWS - 1 - row }
}

/** ICCS 串（如 `h2`）→ 棋理坐标。 */
export function iccsToSquare(iccs: string): Point | null {
  if (iccs.length !== 2) return null
  const col = iccs.charCodeAt(0) - 97 // 'a'
  const row = iccs.charCodeAt(1) - 48 // '0'
  if (col < 0 || col >= COLS || row < 0 || row >= ROWS) return null
  return { col, row }
}

/** 棋理坐标 → ICCS 串。 */
export function squareToIccs(col: number, row: number): string {
  return String.fromCharCode(97 + col) + String(row)
}

// --------------------------------------------------------------- 自检
//
// 90 格的坐标往返必须在开发期就被验证。这类 bug 在生产环境里的表现是
// 「棋子位置整体偏移一格」或「红黑颠倒」—— 肉眼很容易看成设计如此。
if (import.meta.env.DEV) {
  let failures = 0
  for (let col = 0; col < COLS; col += 1) {
    for (let row = 0; row < ROWS; row += 1) {
      const p = squareToPercent(col, row)
      const back = pointToSquare(p.x / 100, p.y / 100)
      if (!back || back.col !== col || back.row !== row) {
        failures += 1
        console.error(`[coords] 坐标往返失败于 (col=${col}, row=${row}) →`, back)
      }
      const flipped = flipSquare(col, row)
      const unflipped = flipSquare(flipped.col, flipped.row)
      if (unflipped.col !== col || unflipped.row !== row) {
        failures += 1
        console.error(`[coords] 翻转不是对合于 (col=${col}, row=${row})`)
      }
    }
  }
  if (failures === 0) {
    console.info('[coords] 90 格坐标往返与翻转对合自检通过')
  }
}
