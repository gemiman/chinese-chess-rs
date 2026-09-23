import { useState } from 'react'

import { Board } from '../components/Board'
import { ClockBar } from '../components/ClockBar'
import { NoteCard, formatScore } from '../components/NoteCard'
import { TacticReveal } from '../components/TacticReveal'
import { navigate } from '../router'
import { legalTargetsFrom, useGameStore } from '../store'
import type { StateDto } from '../types'

/**
 * 对局页。
 *
 * # 一屏之内，不滚动
 *
 * 棋盘、上下两条钟、一行按钮 —— 加起来刚好一屏。这是硬约束：下棋时还要滚屏
 * 才能看到自己的时间，是没法用的。棋盘尺寸由 `--board-width` 按视口高度反推，
 * 见 `styles.css` 里那个变量的说明。
 *
 * # 复盘模式
 *
 * 从终局分析页点「复盘」回到这里时，盘面停在过去的某一手。此时：
 * 落子、悔棋、认输都不可用（Rust 侧也会拒绝，见 `Game::seek`），
 * 棋盘下方那条换成「复盘条」—— 前后翻页 + 这一手的讲解。
 *
 * 两条钟在复盘模式下都不再显示读秒：它们停在终局的残余时间上，
 * 拿它配第 10 手的局面是**错的**，不如明说这是复盘。
 */

/** 底部浮层。三件事共用一层，避免同时弹两个把棋盘挡没了。 */
type Panel = 'none' | 'hint' | 'notation' | 'resign'

export function PlayPage({ state }: { state: StateDto }) {
  const selected = useGameStore((s) => s.selected)
  const flipped = useGameStore((s) => s.flipped)
  const mode = useGameStore((s) => s.mode)
  const playerColor = useGameStore((s) => s.playerColor)
  const thinking = useGameStore((s) => s.thinking)
  const busy = useGameStore((s) => s.busy)
  const moveSpeed = useGameStore((s) => s.moveSpeed)
  const hints = useGameStore((s) => s.hints)
  const hintInfo = useGameStore((s) => s.hintInfo)
  const coachNote = useGameStore((s) => s.coachNote)
  const clickSquare = useGameStore((s) => s.clickSquare)
  const playText = useGameStore((s) => s.playText)
  const playMove = useGameStore((s) => s.playMove)
  const undo = useGameStore((s) => s.undo)
  const resign = useGameStore((s) => s.resign)
  const seek = useGameStore((s) => s.seek)
  const settleTimeout = useGameStore((s) => s.settleTimeout)
  const toggleFlip = useGameStore((s) => s.toggleFlip)
  const requestHint = useGameStore((s) => s.requestHint)
  const clearHint = useGameStore((s) => s.clearHint)

  const [panel, setPanel] = useState<Panel>('none')
  const [text, setText] = useState('')

  const total = state.history.length
  const legalTargets = legalTargetsFrom(state, selected)
  const myTurn = mode === 'hotseat' || state.side === playerColor

  /**
   * 「复盘视图」= 对局已经结束。
   *
   * # 为什么不看游标
   *
   * 一开始我用「游标 < 记录长度」来判断，结果点「复盘」进来时**什么也没发生**：
   * 终局的游标本来就在最后一手上，条件不成立，于是显示的还是那张带
   * 悔棋/认输的对局页，也就没有前后翻手的入口。
   *
   * 而且从语义上「已终局」本来就更准确：棋已经下完了，这一页剩下的唯一用途
   * 就是看棋。悔棋、认输、提示这时都没有意义（超时判负原本能悔棋，
   * 但那条路径现在被终局自动跳转盖住了，留着反而更容易被误用）。
   */
  const ended = state.status.over

  // 复盘视图下棋盘只读：`clickSquare` 里也拦了一道，这里再拦一道是为了
  // 让棋盘连「可走」的提示都不显示 —— 看着能走却点不动最让人困惑。
  const interactive = !ended && myTurn && !thinking && !busy

  return (
    <div className="play">
      <ClockBar
        clock={state.clock}
        side={state.side}
        flipped={flipped}
        over={state.status.over}
        reviewing={ended}
        position="top"
        onStepExpired={() => void settleTimeout()}
      />

      <div className="board-holder">
        <Board
          state={state}
          selected={selected}
          legalTargets={legalTargets}
          flipped={flipped}
          onSquare={clickSquare}
          interactive={interactive}
          moveSpeed={moveSpeed}
        />
        {/* 战法名出场只在对局中放，复盘时别炸场 */}
        {ended ? null : <TacticReveal note={coachNote} />}
      </div>

      {ended ? (
        <ReviewBar state={state} onSeek={(ply) => void seek(ply)} />
      ) : (
        <ClockBar
          clock={state.clock}
          side={state.side}
          flipped={flipped}
          over={false}
          reviewing={false}
          position="bottom"
          onStepExpired={() => void settleTimeout()}
        />
      )}

      <div className="play__bar">
        {ended ? (
          <>
            <button type="button" className="btn" onClick={toggleFlip}>
              翻转
            </button>
            <span className="play__spacer" />
            <button type="button" className="btn" onClick={() => navigate('analysis')}>
              返回分析
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              className="btn"
              onClick={() => void undo()}
              disabled={busy || thinking || total === 0}
            >
              悔棋{mode === 'engine' ? '（退两步）' : ''}
            </button>
            <button
              type="button"
              className="btn"
              onClick={() => setPanel(panel === 'hint' ? 'none' : 'hint')}
              disabled={busy || thinking}
            >
              提示
            </button>
            <button
              type="button"
              className="btn"
              onClick={() => setPanel(panel === 'notation' ? 'none' : 'notation')}
              disabled={busy || thinking}
            >
              记谱
            </button>
            <button type="button" className="btn" onClick={toggleFlip}>
              翻转
            </button>
            <span className="play__spacer" />
            <button
              type="button"
              className={`btn btn--danger${panel === 'resign' ? ' btn--armed' : ''}`}
              onClick={() => setPanel(panel === 'resign' ? 'none' : 'resign')}
            >
              认输
            </button>
          </>
        )}
      </div>

      {panel === 'none' ? null : (
        <div className="sheet" role="dialog" aria-label={PANEL_TITLE[panel]}>
          <div className="sheet__head">
            <span className="sheet__title">{PANEL_TITLE[panel]}</span>
            <button type="button" className="sheet__close" onClick={() => setPanel('none')}>
              关闭
            </button>
          </div>

          {panel === 'hint' ? (
            <HintPanel
              hints={hints}
              hintInfo={hintInfo}
              busy={busy}
              onRequest={() => void requestHint()}
              onClear={clearHint}
              onPlay={(from, to) => {
                setPanel('none')
                void playMove(from, to)
              }}
            />
          ) : null}

          {panel === 'notation' ? (
            <NotationPanel
              text={text}
              onText={setText}
              disabled={thinking}
              onSubmit={() => {
                void playText(text)
                setText('')
                setPanel('none')
              }}
            />
          ) : null}

          {panel === 'resign' ? (
            <ResignPanel
              /** 双人同机时认输的是当前该走的一方，先把名字说清楚再让人点。 */
              loserLabel={mode === 'engine' ? mySideLabel(playerColor) : mySideLabel(state.side)}
              busy={busy}
              onConfirm={() => {
                setPanel('none')
                void resign()
              }}
            />
          ) : null}
        </div>
      )}

      {/* 对局中不放讲解卡。
          对局页的定位是「只显示对局信息」，讲解属于复盘要看的东西；
          而且它要占掉 70 像素，棋盘就只能更小一圈。战法名出场特效照旧会放。
          想看讲解：结束后到分析页，或在复盘模式里一手一手翻。 */}
    </div>
  )
}

const PANEL_TITLE: Record<Exclude<Panel, 'none'>, string> = {
  hint: '走棋提示',
  notation: '按记谱走棋',
  resign: '确认认输',
}

function mySideLabel(color: string): string {
  return color === 'red' ? '红方' : '黑方'
}

/**
 * 复盘条：前后翻手数，外加当前这一手的讲解。
 *
 * ⚠️ 翻页按钮做成**只有图标**，是有原因的：这条与棋盘同宽，而棋盘宽度是按
 * 视口高度反推的 —— 小窗口下只有 330 像素左右。第一版按钮上带「上一手」这样的
 * 文字，五个控件加起来 320 像素起步，直接把这行挤到换行、把「返回分析」顶出
 * 视口（对局页 overflow:hidden，看不见也没滚动条，属于静默失效）。
 * 图标 + `title` 与 `aria-label` 既能塞进一行，也不丢可访问性。
 */
function ReviewBar({ state, onSeek }: { state: StateDto; onSeek: (ply: number) => void }) {
  const notes = useGameStore((s) => s.notes)
  const cursor = state.cursor
  const total = state.history.length
  const note = cursor === 0 ? null : (notes[cursor] ?? null)
  const move = cursor === 0 ? null : state.history[cursor - 1]

  return (
    <div className="review">
      <div className="review__row">
        <button
          type="button"
          className="review__nav"
          title="回到开局"
          aria-label="回到开局"
          onClick={() => onSeek(0)}
          disabled={cursor === 0}
        >
          ⏮
        </button>
        <button
          type="button"
          className="review__nav"
          title="上一手"
          aria-label="上一手"
          onClick={() => onSeek(cursor - 1)}
          disabled={cursor === 0}
        >
          ◀
        </button>
        <span className="review__count">
          第 <strong>{cursor}</strong> / {total} 手
          {move !== null ? <span className="review__move">{move.notation}</span> : null}
        </span>
        <button
          type="button"
          className="review__nav"
          title="下一手"
          aria-label="下一手"
          onClick={() => onSeek(cursor + 1)}
          disabled={cursor >= total}
        >
          ▶
        </button>
        <button
          type="button"
          className="review__nav"
          title="跳到终局"
          aria-label="跳到终局"
          onClick={() => onSeek(total)}
          disabled={cursor >= total}
        >
          ⏭
        </button>
      </div>
      <div className="review__note">
        <NoteCard
          note={note}
          info={null}
          compact
          empty={
            cursor === 0
              ? '这是开局，还没走子。'
              : '这一手还没有讲解 —— 分析页里点「深度分析」可以补全。'
          }
        />
      </div>
    </div>
  )
}

function HintPanel({
  hints,
  hintInfo,
  busy,
  onRequest,
  onClear,
  onPlay,
}: {
  hints: { from: string; to: string; iccs: string; notation: string; score: number; capture: boolean }[]
  hintInfo: { depth: number; nodes: number; think_ms: number } | null
  busy: boolean
  onRequest: () => void
  onClear: () => void
  onPlay: (from: string, to: string) => void
}) {
  if (hints.length === 0) {
    return (
      <>
        <p className="muted" style={{ marginTop: 0 }}>
          让引擎按「大师」档算一算，给三个候选着法。点其中一条就直接走。
        </p>
        <button type="button" className="btn btn--primary" onClick={onRequest} disabled={busy}>
          {busy ? '计算中…' : '求引擎推荐'}
        </button>
      </>
    )
  }
  return (
    <>
      <div className="hints">
        {hints.map((hint) => (
          <button
            key={hint.iccs}
            type="button"
            className={`hint-chip${hint.capture ? ' hint-chip--capture' : ''}`}
            onClick={() => onPlay(hint.from, hint.to)}
            title={`${hint.notation} → ${hint.to}`}
          >
            {hint.notation}
            {hint.capture ? '（吃）' : ''}
            <span className="hint-chip__score">{formatScore(hint.score)}</span>
          </button>
        ))}
      </div>
      {hintInfo !== null ? (
        <p className="engine-line">
          搜索 {hintInfo.depth} 层 · {hintInfo.nodes.toLocaleString()} 节点 · {hintInfo.think_ms} ms
          （评分单位为厘兵，100 = 一个兵）
        </p>
      ) : null}
      <button type="button" className="btn" onClick={onClear} style={{ marginTop: 10 }}>
        收起
      </button>
    </>
  )
}

function NotationPanel({
  text,
  onText,
  disabled,
  onSubmit,
}: {
  text: string
  onText: (v: string) => void
  disabled: boolean
  onSubmit: () => void
}) {
  return (
    <form
      className="notation-form"
      onSubmit={(event) => {
        event.preventDefault()
        onSubmit()
      }}
    >
      <label className="sr-only" htmlFor="notation-input">
        输入中文记谱或坐标
      </label>
      <input
        id="notation-input"
        value={text}
        onChange={(event) => onText(event.target.value)}
        placeholder="如 炮二平五 / 馬8进7 / h2e2"
        autoComplete="off"
        spellCheck={false}
        disabled={disabled}
      />
      <button type="submit" className="btn" disabled={disabled || text.trim().length === 0}>
        走
      </button>
      <p className="muted" style={{ margin: '8px 0 0' }}>
        红方用汉字数字、黑方用阿拉伯数字。也可以直接输 <code>h2e2</code> 这样的坐标。
      </p>
    </form>
  )
}

function ResignPanel({
  loserLabel,
  busy,
  onConfirm,
}: {
  loserLabel: string
  busy: boolean
  onConfirm: () => void
}) {
  return (
    <>
      <p style={{ marginTop: 0 }}>
        认输之后这一局就结束了，<strong>不能撤消</strong>。确认要由
        <strong>{loserLabel}</strong>认输吗？
      </p>
      <p className="muted">结束后会自动跳到分析页，那里可以复盘这一局。</p>
      <button type="button" className="btn btn--danger" onClick={onConfirm} disabled={busy}>
        确认认输
      </button>
    </>
  )
}
