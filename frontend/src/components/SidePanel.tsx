import { useState } from 'react'

import { useGameStore } from '../store'
import { DIFFICULTIES, THINK_MS, type Color, type GameMode, type MoveOption, type StateDto } from '../types'

interface SidePanelProps {
  state: StateDto
  selected: string | null
  legalTargets: MoveOption[]
  busy: boolean
  flipped: boolean
  /** 提交一步记谱或坐标。 */
  onPlay: (text: string) => void
  /** 直接走某个合法落点（点提示条用）。 */
  onPickSquare: (sq: string) => void
  /** 直接走一步（点推荐着法条用）。 */
  onMove: (from: string, to: string) => void
  onUndo: () => void
  onReset: () => void
  onFlip: () => void
}

const SIDE_LABEL: Record<string, string> = { red: '红方', black: '黑方' }

export function SidePanel(props: SidePanelProps) {
  const { state, selected, legalTargets, busy, flipped, onPlay, onPickSquare, onMove, onUndo, onReset, onFlip } =
    props
  const [text, setText] = useState('')
  const [copied, setCopied] = useState(false)

  const mode = useGameStore((s) => s.mode)
  const playerColor = useGameStore((s) => s.playerColor)
  const difficulty = useGameStore((s) => s.difficulty)
  const thinking = useGameStore((s) => s.thinking)
  const engineInfo = useGameStore((s) => s.engineInfo)
  const hints = useGameStore((s) => s.hints)
  const hintInfo = useGameStore((s) => s.hintInfo)
  const setMode = useGameStore((s) => s.setMode)
  const setPlayerColor = useGameStore((s) => s.setPlayerColor)
  const setDifficulty = useGameStore((s) => s.setDifficulty)
  const requestHint = useGameStore((s) => s.requestHint)
  const clearHint = useGameStore((s) => s.clearHint)

  const turnLabel = SIDE_LABEL[state.side] ?? state.side
  const statusTone = state.status.over ? 'over' : state.in_check ? 'check' : state.side
  const selectedPiece = selected === null ? null : state.pieces.find((p) => p.sq === selected)

  async function copyFen() {
    try {
      await navigator.clipboard.writeText(state.fen)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1600)
    } catch {
      setCopied(false)
    }
  }

  // 把着法记录按「回合」两两成对
  const rows: { no: number; red?: string; black?: string; redPly?: number; blackPly?: number }[] = []
  state.history.forEach((move, index) => {
    const ply = index + 1
    const round = Math.floor(index / 2)
    if (index % 2 === 0) {
      rows.push({ no: round + 1, red: move.notation, redPly: ply })
    } else {
      rows[round].black = move.notation
      rows[round].blackPly = ply
    }
  })
  const lastPly = state.history.length
  const activeDifficulty = DIFFICULTIES.find((d) => d.id === difficulty)

  return (
    <div className="panel">
      <section className="card">
        <div className="status">
          <span className={`status__dot status__dot--${statusTone}`} aria-hidden="true" />
          <span className="status__text">{state.status.text}</span>
        </div>
        <div className="status__meta">
          轮到 {turnLabel} 走 · 共 {state.legal.length} 种合法着法 · 第 {state.fullmove_number} 回合 ·
          半回合计数 {state.halfmove_clock}
        </div>
      </section>

      <CoachCard />

      <section className="card">
        <h2 className="card__title">对局模式</h2>
        <div className="segmented" role="group" aria-label="对局模式">
          {(
            [
              { id: 'hotseat', label: '双人同机' },
              { id: 'engine', label: '人机对战' },
            ] as { id: GameMode; label: string }[]
          ).map((item) => (
            <button
              key={item.id}
              type="button"
              className={`segmented__item${mode === item.id ? ' segmented__item--active' : ''}`}
              aria-pressed={mode === item.id}
              onClick={() => setMode(item.id)}
            >
              {item.label}
            </button>
          ))}
        </div>

        {mode === 'engine' ? (
          <>
            <div className="field">
              <span className="field__label">我执</span>
              <div className="segmented" role="group" aria-label="我执哪一方">
                {(['red', 'black'] as Color[]).map((color) => (
                  <button
                    key={color}
                    type="button"
                    className={`segmented__item${
                      playerColor === color ? ' segmented__item--active' : ''
                    }`}
                    aria-pressed={playerColor === color}
                    onClick={() => setPlayerColor(color)}
                  >
                    {SIDE_LABEL[color]}
                  </button>
                ))}
              </div>
            </div>

            <div className="field">
              <span className="field__label">引擎棋力</span>
              <div className="segmented segmented--wrap" role="group" aria-label="引擎棋力">
                {DIFFICULTIES.map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    className={`segmented__item${
                      difficulty === item.id ? ' segmented__item--active' : ''
                    }`}
                    aria-pressed={difficulty === item.id}
                    title={`${item.label} · ${item.subtitle}`}
                    onClick={() => setDifficulty(item.id)}
                  >
                    {item.label}
                  </button>
                ))}
              </div>
              {activeDifficulty ? (
                <p className="muted" style={{ margin: '6px 0 0' }}>
                  {activeDifficulty.subtitle} · 每步思考约 {THINK_MS[difficulty]} 毫秒
                </p>
              ) : null}
            </div>

            {engineInfo !== null ? (
              <p className="engine-line">
                引擎（{engineInfo.level_label}）深度 {engineInfo.depth} ·
                评分 {formatScore(engineInfo.score)} · {engineInfo.nodes.toLocaleString()} 节点 ·{' '}
                {engineInfo.think_ms} ms
                {engineInfo.mate_in !== null ? ` · ${mateText(engineInfo.mate_in)}` : ''}
                {engineInfo.stopped ? ' · 时间用尽' : ''}
              </p>
            ) : null}
          </>
        ) : (
          <p className="muted" style={{ marginBottom: 0 }}>
            双方在同一块棋盘上轮流走。切到「人机对战」可以和 Rust 引擎对局。
          </p>
        )}
      </section>

      <section className="card">
        <h2 className="card__title">走棋提示</h2>
        <div className="actions" style={{ marginBottom: 10 }}>
          <button
            type="button"
            className="btn"
            onClick={() => void requestHint()}
            disabled={busy || thinking || state.status.over}
          >
            求引擎推荐
          </button>
          {hints.length > 0 ? (
            <button type="button" className="btn" onClick={clearHint}>
              收起
            </button>
          ) : null}
        </div>

        {hints.length > 0 ? (
          <>
            <div className="hints">
              {hints.map((hint) => (
                <button
                  key={hint.iccs}
                  type="button"
                  className={`hint-chip${hint.capture ? ' hint-chip--capture' : ''}`}
                  onClick={() => onMove(hint.from, hint.to)}
                  title={`${hint.notation} → ${hint.to}（评分 ${hint.score}）`}
                >
                  {hint.notation}
                  {hint.capture ? '（吃）' : ''}
                  <span className="hint-chip__score">{formatScore(hint.score)}</span>
                </button>
              ))}
            </div>
            {hintInfo !== null ? (
              <p className="engine-line">
                按「大师」档搜索 {hintInfo.depth} 层 · {hintInfo.nodes.toLocaleString()} 节点 ·{' '}
                {hintInfo.think_ms} ms（评分单位为厘兵，100 = 一个兵）
              </p>
            ) : null}
          </>
        ) : selected === null ? (
          <p className="muted" style={{ marginBottom: 0 }}>
            点棋盘上的棋子看它能走到哪里；或点上面的按钮让引擎给三个候选着法。
          </p>
        ) : (
          <>
            <p className="muted">
              {SIDE_LABEL[selectedPiece?.color ?? state.side]}
              {selectedPiece?.glyph} 位于 <code>{selected}</code>，可走 {legalTargets.length} 步
              （带「吃」的会吃掉对方棋子）：
            </p>
            <div className="hints">
              {legalTargets.length === 0 ? (
                <span className="muted">这枚棋子无处可走。</span>
              ) : (
                legalTargets.map((move) => (
                  <button
                    key={move.iccs}
                    type="button"
                    className={`hint-chip${move.capture ? ' hint-chip--capture' : ''}`}
                    onClick={() => onPickSquare(move.to)}
                    title={`${move.notation} → ${move.to}`}
                  >
                    {move.notation}
                    {move.capture ? '（吃）' : ''}
                  </button>
                ))
              )}
            </div>
          </>
        )}
      </section>

      <section className="card">
        <h2 className="card__title">操作</h2>
        <div className="actions">
          <button
            type="button"
            className="btn"
            onClick={onUndo}
            disabled={busy || thinking || state.history.length === 0}
          >
            悔棋{mode === 'engine' ? '（退两步）' : ''}
          </button>
          <button type="button" className="btn" onClick={onReset} disabled={busy || thinking}>
            重开
          </button>
          <button type="button" className="btn" onClick={onFlip}>
            {flipped ? '黑方视角' : '红方视角'} · 翻转
          </button>
          <button type="button" className="btn" onClick={copyFen} disabled={busy}>
            {copied ? '已复制 FEN' : '复制 FEN'}
          </button>
        </div>
      </section>

      <section className="card">
        <h2 className="card__title">按记谱走棋</h2>
        <form
          className="notation-form"
          onSubmit={(event) => {
            event.preventDefault()
            onPlay(text)
            setText('')
          }}
        >
          <label className="sr-only" htmlFor="notation-input">
            输入中文记谱或坐标
          </label>
          <input
            id="notation-input"
            value={text}
            onChange={(event) => setText(event.target.value)}
            placeholder="如 炮二平五 / 马8进7 / h2e2"
            autoComplete="off"
            spellCheck={false}
            disabled={thinking}
          />
          <button type="submit" className="btn" disabled={busy || thinking || text.trim().length === 0}>
            走
          </button>
        </form>
        <p className="muted" style={{ marginTop: 8, marginBottom: 0 }}>
          红方用汉字数字、黑方用阿拉伯数字。也可以直接输 <code>h2e2</code> 这样的坐标。
        </p>
      </section>

      <section className="card">
        <h2 className="card__title">着法记录</h2>
        {rows.length === 0 ? (
          <p className="muted">尚未走子。</p>
        ) : (
          <ol className="moves">
            {rows.map((row) => (
              <li key={row.no} className="moves__row">
                <span className="moves__no">{row.no}.</span>
                <span className={`moves__red${row.redPly === lastPly ? ' moves__last' : ''}`}>
                  {row.red ?? ''}
                </span>
                <span className={`moves__black${row.blackPly === lastPly ? ' moves__last' : ''}`}>
                  {row.black ?? ''}
                </span>
              </li>
            ))}
          </ol>
        )}
      </section>

      <section className="card">
        <h2 className="card__title">标记说明</h2>
        <div className="legend">
          <div className="legend__item">
            <span className="legend__swatch">
              <span className="legend__dot" />
            </span>
            实心圆点 = 该处为空，可以走过去
          </div>
          <div className="legend__item">
            <span className="legend__swatch">
              <span className="legend__ring" />
            </span>
            空心圆环 = 该处有对方棋子，可以吃掉
          </div>
          <div className="legend__item">
            <span className="legend__swatch">
              <span className="legend__sel" />
            </span>
            琥珀色环 = 当前选中的棋子
          </div>
          <div className="legend__item">
            <span className="legend__swatch">
              <span className="legend__last" />
            </span>
            方角标 = 上一着的起点与终点
          </div>
        </div>
        <p className="muted" style={{ marginBottom: 0 }}>
          每种状态都用「颜色 + 形状」双重区分 —— 仅靠颜色在木色棋盘上对比度不足。
        </p>
      </section>

      <section className="card">
        <h2 className="card__title">当前局面 FEN</h2>
        <p className="fen">{state.fen}</p>
      </section>
    </div>
  )
}

/** 把厘兵评分格式化成带正负号的形式。 */
function formatScore(score: number): string {
  const sign = score > 0 ? '+' : ''
  return `${sign}${(score / 100).toFixed(2)}`
}

/** 杀棋距离的中文描述。 */
function mateText(distance: number): string {
  return distance > 0 ? `我方 ${distance} 步内将死` : `我方 ${-distance} 步内被将死`
}

/**
 * 战法讲解卡片。
 *
 * # 等级展示的三重编码（强制约束）
 *
 * `assets/tokens/colors.md` 的 WCAG 实算结论：等级色在浅色底上对比度普遍不达标
 * （`coach.best` 只有 2.87:1）。因此等级**不允许只用颜色传达** ——
 * 本卡片同时给出：颜色底（徽标背景）+ 图标形状（★ ✓ ? ✕ ‼）+ 文字标签。
 */
function CoachCard() {
  const note = useGameStore((s) => s.coachNote)
  const info = useGameStore((s) => s.coachInfo)
  const loading = useGameStore((s) => s.coachLoading)

  if (note === null) {
    return (
      <section className="card">
        <h2 className="card__title">战法讲解</h2>
        <p className="muted" style={{ marginBottom: 0 }}>
          {loading ? '正在生成讲解…' : '走一步棋，这里会给出这步的战法名称与评价。'}
        </p>
      </section>
    )
  }

  return (
    <section className="card">
      <h2 className="card__title">
        战法讲解
        {loading ? <span className="muted"> · 更新中…</span> : null}
      </h2>

      <div className={`note lv-${note.level}`}>
        <div className="note__head">
          <span className={`lv-badge lv-${note.level}`}>
            <span className="lv-badge__glyph" aria-hidden="true">
              {LEVEL_GLYPH[note.level]}
            </span>
            {LEVEL_LABEL[note.level]}
          </span>
          <span className="note__move">{note.notation}</span>
          {note.score_loss > 0 ? (
            <span className="note__loss">分差 {formatScore(note.score_loss)}</span>
          ) : null}
        </div>

        <p className="note__headline">{note.headline}</p>
        {note.detail ? <p className="note__detail">{note.detail}</p> : null}

        {note.tactics.length > 0 ? (
          <div className="hints" style={{ marginTop: 8 }}>
            {note.tactics.map((tactic) => (
              <span
                key={tactic.id}
                className={`tactic-chip tactic-chip--${tactic.category}${
                  tactic.confidence < 0.7 ? ' tactic-chip--weak' : ''
                }`}
                title={`${CATEGORY_LABEL[tactic.category]} · 置信度 ${(tactic.confidence * 100).toFixed(0)}%`}
              >
                {tactic.name}
              </span>
            ))}
          </div>
        ) : null}

        {note.opening_name !== null ? (
          <p className="note__meta">开局：{note.opening_name}</p>
        ) : null}

        {note.suggestion !== null ? (
          <p className="note__suggestion">{note.suggestion}</p>
        ) : null}

        {note.used_fallback ? (
          <p className="note__meta">
            （这条讲解走了兜底模板 —— 说明该着法暂时没有匹配到合适的战术模板）
          </p>
        ) : null}

        {info !== null ? (
          <p className="note__meta">
            评价依据：引擎搜索 {info.depth} 层 · {info.nodes.toLocaleString()} 节点 ·{' '}
            {info.think_ms} ms（分差单位为厘兵，100 = 一个兵）
          </p>
        ) : null}
      </div>
    </section>
  )
}

const LEVEL_LABEL: Record<string, string> = {
  best: '最佳着法',
  good: '不错',
  dubious: '稍有问题',
  blunder: '明显失误',
  missed: '严重漏着',
}

/** 等级图标 —— 形状各异，供色觉障碍用户区分。绝不能只用颜色。 */
const LEVEL_GLYPH: Record<string, string> = {
  best: '★',
  good: '✓',
  dubious: '?',
  blunder: '✕',
  missed: '‼',
}

const CATEGORY_LABEL: Record<string, string> = {
  structure: '结构层（确定）',
  relation: '关系层',
  formation: '棋形层（待校准）',
  opening: '开局',
}
