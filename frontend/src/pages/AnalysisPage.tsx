import { LEVEL_LABEL, NoteCard } from '../components/NoteCard'
import { navigate } from '../router'
import { useGameStore } from '../store'
import { DEEP_THINK_MS, type StateDto } from '../types'

/**
 * 终局分析页。
 *
 * # 为什么和对局页分开
 *
 * 一局棋打完，人想看的东西完全变了：不再是「我该走哪」，而是「刚才那几步
 * 到底怎么样」。这两件事需要的界面没法共存 —— 硬塞在一页里，就会变成
 * 对局时被棋谱挤，复盘时被棋盘挤。所以分出这一页。
 *
 * # 讲解有两个来源（用户选的「两者都要」）
 *
 * 1. **下棋时顺手攒的**：每走一步引擎就会生成一条讲解，以前被下一步覆盖掉了，
 *    现在按手数存下来。进这一页立刻就有内容，零等待。
 * 2. **赛后逐手重算**：点「深度分析」，让引擎从第一手重算一遍。
 *    慢（每手几百毫秒 × 手数），但完整、且评价标准统一。
 *
 * 页面上把两者分得很清楚：列表里已有的先显示，重算过的会带一个「重算」标记。
 */

const SIDE_LABEL: Record<string, string> = { red: '红方', black: '黑方' }

export function AnalysisPage({ state }: { state: StateDto }) {
  const notes = useGameStore((s) => s.notes)
  const deep = useGameStore((s) => s.deep)
  const seek = useGameStore((s) => s.seek)
  const runDeepAnalysis = useGameStore((s) => s.runDeepAnalysis)
  const cancelDeep = useGameStore((s) => s.cancelDeep)

  const loser = state.status.loser
  const winner = loser === null ? null : loser === 'red' ? 'black' : 'red'
  const verdict = winner === null ? '和棋' : `${SIDE_LABEL[winner]}胜`

  const captures = state.history.filter((m) => m.capture).length
  const levels = state.history.map((_, index) => notes[index + 1]?.level)
  const counted = levels.filter((lv) => lv !== undefined)
  const opening = Object.keys(notes)
    .map(Number)
    .sort((a, b) => a - b)
    .map((ply) => notes[ply].opening_name)
    .find((name) => name !== null)

  const remainingMs = Math.max(0, (deep.total - deep.done) * DEEP_THINK_MS)

  /** 跳到某一手看棋。先切页再定位 —— 定位要等一下 Rust，不该让用户干等白屏。 */
  function openReview(ply: number) {
    navigate('play')
    void seek(ply)
  }

  return (
    <div className="analysis">
      <section className={`verdict verdict--${winner ?? 'draw'}`}>
        <span className="verdict__label">本局结果</span>
        <h2 className="verdict__headline">{verdict}</h2>
        <p className="verdict__text">{state.status.text}</p>
      </section>

      <section className="stats">
        <Stat label="总手数" value={`${state.history.length} 手`} />
        <Stat label="吃子" value={`${captures} 次`} />
        <Stat label="已讲解" value={`${counted.length} / ${state.history.length} 手`} />
        <Stat
          label="失误"
          value={`${counted.filter((lv) => lv === 'blunder' || lv === 'missed').length} 手`}
        />
        {opening !== null && opening !== undefined ? <Stat label="开局" value={opening} /> : null}
      </section>

      <section className="actions analysis__actions">
        <button type="button" className="btn btn--primary" onClick={() => openReview(state.history.length)}>
          复盘
        </button>
        {deep.running ? (
          <button type="button" className="btn" onClick={cancelDeep}>
            中止分析
          </button>
        ) : (
          <button
            type="button"
            className="btn"
            onClick={() => void runDeepAnalysis()}
            disabled={state.history.length === 0}
          >
            {deep.done > 0 && deep.done < state.history.length ? '继续深度分析' : '深度分析'}
          </button>
        )}
        <button type="button" className="btn" onClick={() => navigate('setup')}>
          再来一局
        </button>
      </section>

      {deep.running || deep.done > 0 || deep.error !== null ? (
        <section className="deep">
          <div className="deep__head">
            <span className="deep__label">
              深度分析 {deep.done} / {deep.total} 手
              {deep.running ? ` · 约剩 ${Math.ceil(remainingMs / 1000)} 秒` : ''}
            </span>
          </div>
          <div className="deep__track" role="progressbar" aria-valuenow={deep.done} aria-valuemin={0} aria-valuemax={deep.total}>
            <div
              className="deep__fill"
              style={{ width: `${deep.total === 0 ? 0 : (deep.done / deep.total) * 100}%` }}
            />
          </div>
          {deep.error !== null ? (
            <p className="muted deep__error">
              分析中断：{deep.error}
              <br />
              已经算完的手数保留着，再点一次「继续深度分析」会接着往下算。
            </p>
          ) : null}
          {deep.running ? (
            <p className="muted deep__error">
              每手让引擎思考 {DEEP_THINK_MS} 毫秒。可以离开这一页去做别的，进度不会丢。
            </p>
          ) : null}
        </section>
      ) : null}

      <section className="card">
        <h3 className="card__title">逐手讲解 · 点任意一手跳到那一步的盘面</h3>
        {state.history.length === 0 ? (
          <p className="muted" style={{ marginBottom: 0 }}>这一局还没走过子。</p>
        ) : (
          <ol className="movelist">
            {state.history.map((move, index) => {
              const ply = index + 1
              const note = notes[ply]
              return (
                <li key={move.iccs + ply} className="movelist__row">
                  <button
                    type="button"
                    className="movelist__jump"
                    onClick={() => openReview(ply)}
                    title={`查看第 ${ply} 手的盘面`}
                  >
                    <span className="movelist__no">{ply}</span>
                    <span className={`movelist__side movelist__side--${move.side}`}>
                      {SIDE_LABEL[move.side]}
                    </span>
                    <span className="movelist__notation">
                      {move.notation}
                      {move.capture ? <span className="movelist__cap">吃</span> : null}
                    </span>
                  </button>
                  <div className="movelist__body">
                    {note === undefined ? (
                      <p className="muted" style={{ margin: 0 }}>
                        还没有讲解 —— 点上面的「深度分析」可以补全这一手。
                      </p>
                    ) : (
                      <>
                        <span className={`lv-badge lv-${note.level}`}>
                          {LEVEL_LABEL[note.level]}
                        </span>
                        <span className="movelist__headline">{note.headline}</span>
                      </>
                    )}
                  </div>
                </li>
              )
            })}
          </ol>
        )}
      </section>

      <section className="card">
        <h3 className="card__title">当前局面 FEN</h3>
        <p className="fen">{state.fen}</p>
      </section>

      {/* 最后一步的完整讲解（含建议与依据）放在最下面，想细看的人往下拉 */}
      {notes[state.history.length] !== undefined ? (
        <section className="card">
          <h3 className="card__title">终局那一手的完整讲解</h3>
          <NoteCard note={notes[state.history.length]} />
        </section>
      ) : null}
    </div>
  )
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="stat">
      <span className="stat__label">{label}</span>
      <span className="stat__value">{value}</span>
    </div>
  )
}
