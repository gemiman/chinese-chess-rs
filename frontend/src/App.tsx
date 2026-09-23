import { useEffect } from 'react'

import { Board } from './components/Board'
import { SidePanel } from './components/SidePanel'
import { runningInTauri } from './bridge'
import { legalTargetsFrom, useGameStore } from './store'

export default function App() {
  const state = useGameStore((s) => s.state)
  const selected = useGameStore((s) => s.selected)
  const error = useGameStore((s) => s.error)
  const busy = useGameStore((s) => s.busy)
  const flipped = useGameStore((s) => s.flipped)
  const load = useGameStore((s) => s.load)
  const clickSquare = useGameStore((s) => s.clickSquare)
  const playText = useGameStore((s) => s.playText)
  const playMove = useGameStore((s) => s.playMove)
  const undo = useGameStore((s) => s.undo)
  const reset = useGameStore((s) => s.reset)
  const toggleFlip = useGameStore((s) => s.toggleFlip)
  const dismissError = useGameStore((s) => s.dismissError)

  const mode = useGameStore((s) => s.mode)
  const playerColor = useGameStore((s) => s.playerColor)
  const thinking = useGameStore((s) => s.thinking)
  const enginePlay = useGameStore((s) => s.enginePlay)

  useEffect(() => {
    void load()
  }, [load])

  // 轮到引擎时自动走一步。
  //
  // 重复触发由 store 里的 `engineInFlight` 同步守卫挡住 —— 只靠 `thinking`
  // 标志不够，因为 set() 到组件重渲染之间有一次微任务，StrictMode 会把副作用
  // 跑两遍，导致引擎连走两步。
  useEffect(() => {
    if (mode !== 'engine' || state === null || state.status.over || thinking) return
    if (state.side === playerColor) return
    void enginePlay()
  }, [mode, state, playerColor, thinking, enginePlay])

  if (state === null) {
    return <BootScreen error={error} busy={busy} onRetry={() => void load()} />
  }

  const legalTargets = legalTargetsFrom(state, selected)
  const playerTurn = mode === 'hotseat' || state.side === playerColor

  return (
    <div className="app">
      <header className="app__header">
        <h1 className="app__title">弈道</h1>
        <span className="app__subtitle">
          中国象棋 · Rust 规则内核 + Rust 搜索引擎
        </span>
        <span className="app__spacer" />
        {thinking ? (
          <span className="app__thinking">
            <span className="spinner" aria-hidden="true" />
            引擎思考中…
          </span>
        ) : busy ? (
          <span className="muted">同步中…</span>
        ) : null}
      </header>

      <main className="app__main">
        <section>
          <Board
            state={state}
            selected={selected}
            legalTargets={legalTargets}
            flipped={flipped}
            onSquare={clickSquare}
            interactive={playerTurn && !thinking}
          />
          {error !== null ? (
            <div className="notice notice--error" style={{ marginTop: 16 }} role="alert">
              <strong>操作失败：</strong>
              {error}
              <button
                type="button"
                className="btn"
                style={{ marginLeft: 12 }}
                onClick={dismissError}
              >
                知道了
              </button>
            </div>
          ) : null}
        </section>

        <aside>
          <SidePanel
            state={state}
            selected={selected}
            legalTargets={legalTargets}
            busy={busy}
            flipped={flipped}
            onPlay={(text) => void playText(text)}
            onPickSquare={clickSquare}
            onMove={(from, to) => void playMove(from, to)}
            onUndo={() => void undo()}
            onReset={() => void reset()}
            onFlip={toggleFlip}
          />
        </aside>
      </main>
    </div>
  )
}

/** 引擎连不上时的引导页。 */
function BootScreen({
  error,
  busy,
  onRetry,
}: {
  error: string | null
  busy: boolean
  onRetry: () => void
}) {
  return (
    <div className="boot">
      <div className="boot__box">
        <h1 className="boot__title">弈道 · 中国象棋</h1>
        {error === null ? (
          <p className="boot__hint">正在连接规则引擎…</p>
        ) : runningInTauri ? (
          <>
            {/* 桌面端直接调 Rust，正常情况下走不到这里；出现即说明 Rust 侧报错 */}
            <p className="boot__hint">桌面端初始化失败。这通常是 Rust 侧的错误，详情见下。</p>
            <details open>
              <summary className="muted">错误详情</summary>
              <pre className="notice__pre">{error}</pre>
            </details>
          </>
        ) : (
          <>
            <p className="boot__hint">
              连不上规则引擎。你现在是在普通浏览器里打开，这种方式需要后端提供走法合法性判定与 AI。
            </p>
            <p className="boot__hint">在项目根目录执行下面这条，然后刷新本页：</p>
            <code className="boot__cmd">cargo run --release -p xq-bridge -- --open</code>
            <p className="muted">
              或者直接跑桌面客户端（不需要这一层）：
              <br />
              <code>cargo run --release -p xq-client</code>
            </p>
            <details>
              <summary className="muted">看具体错误</summary>
              <pre className="notice__pre">{error}</pre>
            </details>
          </>
        )}
        <button type="button" className="btn" onClick={onRetry} disabled={busy}>
          {busy ? '重试中…' : '重试'}
        </button>
      </div>
    </div>
  )
}
