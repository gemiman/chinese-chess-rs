import { useEffect, useRef } from 'react'

import { AnalysisPage } from './pages/AnalysisPage'
import { PlayPage } from './pages/PlayPage'
import { SetupPage } from './pages/SetupPage'
import { runningInTauri } from './bridge'
import { navigate, useRoute, type Route } from './router'
import { useGameStore } from './store'

/**
 * 应用外壳：决定显示哪一页，以及两个「自动跳转」。
 *
 * # 为什么自动跳转写在这里，不写进 store
 *
 * 跳转是**导航**，不是对局状态。store 里塞一个 `navigate()` 会让它依赖路由，
 * 而路由只在这一个文件里有意义（测试、将来换宿主都不用管它）。
 *
 * # 两个自动跳转都有坑，各写在各自的注释里
 *
 * 1. 终局 → 分析页。只能认「刚刚变成终局」这一次，否则从分析页点「复盘」
 *    回到对局页会被立刻弹回去，复盘根本进不去。
 * 2. 刷新后落在对局页 → 设置页。刷新会把 store 丢掉，`started` 变回 false，
 *    这时 `#/play` 后面没有对局可看。
 */
export default function App() {
  const route = useRoute()
  const state = useGameStore((s) => s.state)
  const started = useGameStore((s) => s.started)
  const error = useGameStore((s) => s.error)
  const busy = useGameStore((s) => s.busy)
  const thinking = useGameStore((s) => s.thinking)
  const mode = useGameStore((s) => s.mode)
  const playerColor = useGameStore((s) => s.playerColor)
  const load = useGameStore((s) => s.load)
  const enginePlay = useGameStore((s) => s.enginePlay)
  const dismissError = useGameStore((s) => s.dismissError)

  /** 实际显示的那一页。还没开局时无论网址是什么，看到的都是设置页。 */
  const page: Route = started ? route : 'setup'

  useEffect(() => {
    void load()
  }, [load])

  // 轮到引擎时自动走一步。
  //
  // 重复触发由 store 里的 `engineInFlight` 同步守卫挡住 —— 只靠 `thinking`
  // 标志不够，因为 set() 到组件重渲染之间有一次微任务，StrictMode 会把副作用
  // 跑两遍，导致引擎连走两步。
  useEffect(() => {
    // ⚠️ 必须在**对局页**上才让引擎走。
    // 设置页上「选了人机对战 + 执黑」时，上面那三个条件也全都成立 —— 于是引擎会
    // 在一局**还没开始**的棋里抢先走一步，接着「开始游戏」把局面重置掉，白白多算
    // 一次、还可能中途报错。这个 bug 是桌面端冒烟测试抓出来的：它表现为
    // 「开局之后引擎一动不动」，因为真正的引擎调用早就浪费在设置页上了。
    if (page !== 'play') return
    if (mode !== 'engine' || state === null || state.status.over || thinking) return
    // 复盘时盘面停在过去，此刻的「走子方」是历史，不是轮到谁走
    if (state.cursor !== state.history.length) return
    if (state.side === playerColor) return
    void enginePlay()
  }, [page, mode, state, playerColor, thinking, enginePlay])

  // 终局 → 分析页。注意只能认「刚才还不是终局」这一个瞬间。
  const over = state?.status.over ?? false
  const wasOver = useRef(false)
  useEffect(() => {
    if (over && !wasOver.current) navigate('analysis')
    wasOver.current = over
  }, [over])

  // 没有对局却停在需要局面的页面上（刷新、或手改网址、或桌面端恢复了上次的深链）
  // → 送回设置页。
  //
  // ⚠️ 用 `replace`：这条跳转是**兜底**，不是用户走的。用 push 的话，
  // 回退到 #/play 会被再推一条 #/setup，再回退再推 —— 后退键彻底失灵。
  useEffect(() => {
    if (!started && (route === 'play' || route === 'analysis')) navigate('setup', true)
  }, [started, route])

  if (state === null) {
    return <BootScreen error={error} busy={busy} onRetry={() => void load()} />
  }

  return (
    <div className={`app app--${page}`}>
      <header className="app__header">
        <h1 className="app__title">弈道</h1>
        <span className="app__subtitle">中国象棋 · Rust 规则内核 + Rust 搜索引擎</span>
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

      <main className={`app__main app__main--${page}`}>
        {page === 'play' ? (
          <PlayPage state={state} />
        ) : page === 'analysis' ? (
          <AnalysisPage state={state} />
        ) : (
          <SetupPage />
        )}
      </main>

      {error !== null ? (
        <div className="toast" role="alert">
          <span className="toast__text">{error}</span>
          <button type="button" className="btn" onClick={dismissError}>
            知道了
          </button>
        </div>
      ) : null}
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
