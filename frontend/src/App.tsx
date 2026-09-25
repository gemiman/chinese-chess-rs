import { useEffect, useRef, useState } from 'react'

import { AnalysisPage } from './pages/AnalysisPage'
import { PlayPage } from './pages/PlayPage'
import { SetupPage } from './pages/SetupPage'
import { runningInTauri } from './bridge'
import { REVEAL_MS } from './components/TacticReveal'
import { navigate, useRoute, type Route } from './router'
import { announce, announceTactic, silence } from './speech'
import { useGameStore } from './store'
import { AUTO_PACE_MS, MOVE_MS } from './types'

/**
 * 终局后最多等讲解多久（毫秒）。
 *
 * 讲解要跑一次搜索才出得来；认输、超时这两种终局压根没有讲解，所以这段必须有上限，
 * 不能死等 —— 否则那两种终局跳转就永远不发生了。
 */
const NOTE_WAIT_MS = 3000

/**
 * 认输、超时这两种终局只留这么一点（毫秒），然后就走。
 *
 * 它们是**会话层**的终局，棋盘上没有一步棋，也就永远不会有讲解、不会有出场特效。
 * 硬等着讲解是白等 —— 用户盯着一个已经结束的盘面等三秒，什么也不会发生。
 * 留这一点只是为了让确认框收起来、画面稳一下。
 */
const SETTLE_MS = 400

/**
 * 特效播完之后再留一拍（毫秒）。
 *
 * 牌匾落下来、念完「绝杀」，再停一下才跳页 —— 否则特效刚收尾页面就换掉了，
 * 这一局的最后一眼等于没看清。只在**确实放了特效**时留这一拍：认输、超时没有特效，
 * 那两种终局前面已经白等了一段，再停就没意义了。
 */
const AFTER_REVEAL_MS = 2000

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
  const autoPlaying = useGameStore((s) => s.autoPlaying)
  const load = useGameStore((s) => s.load)
  const enginePlay = useGameStore((s) => s.enginePlay)
  const dismissError = useGameStore((s) => s.dismissError)

  /** 实际显示的那一页。还没开局时无论网址是什么，看到的都是设置页。 */
  const page: Route = started ? route : 'setup'

  useEffect(() => {
    void load()
  }, [load])

  // 机机对战：两步之间的最小间隔。
  //
  // ⚠️ 这件事只能在这里做，**不能改成「让引擎多想一会儿」**。实测：入门档只想
  // 2 层，给它 3000 毫秒预算，它 0 毫秒就算完返回了 —— 弱档位的设计就是「算完
  // 即走」，时间预算对它没有约束力（中级 169 毫秒，只有大师真用满）。所以想让棋
  // 别从眼前飞过去，只能在**节奏**上加下限。
  //
  // ⚠️ 这段必须**排在下面那个自动应招的 effect 前面**。副作用按声明顺序执行，
  // 下面的 effect 要读这里刚写下的 `lastMoveAt`；调换顺序的话它读到的永远是上一手
  // 的时间戳，间隔会莫名其妙地少一拍 —— 而且不报任何错。
  const lastMoveAt = useRef(0)
  const prevSteps = useRef<number | null>(null)
  const steps = state?.history.length ?? 0
  useEffect(() => {
    const prev = prevSteps.current
    prevSteps.current = steps
    // 首次挂载不算；步数变少说明换了一局（或悔棋），基准清零让第一步立刻走
    if (prev === null) return
    lastMoveAt.current = steps > prev ? Date.now() : 0
  }, [steps])

  // ---------------------------------------------------------------- 语音播报
  //
  // 播报的**内容**由 speech.ts 决定（谁先谁后、谁能插队），这里只管
  // **什么时候**让它响。
  const spokenSteps = useRef<number | null>(null)
  /** 本手棋子落定的时刻。战法名比走子晚到，得等它一起落定再响。 */
  const landAt = useRef(0)
  /** 已经念过战法名的那一手。 */
  const tacticPly = useRef(0)

  /**
   * 走子之后播报。
   *
   * ⚠️ 等 `MOVE_MS` 才响，不是收到响应就响：声音要跟**棋子落下来**对上。
   * 动画有快/正常/慢动作三档（180 / 400 / 1000 毫秒），棋子还在半路上就开始
   * 喊「将军」会很怪。
   *
   * ⚠️ 依赖只写 `steps` 和 `page`，其余从 `getState()` 现取。把整个 `state`
   * 放进依赖会让每次快照刷新（结算超时、失败后重拉局面）都重跑一遍，而那时
   * `steps` 没变 —— 于是走进「步数没增加」那条分支，把还没念完的播报清掉。
   */
  useEffect(() => {
    if (page !== 'play') {
      silence()
      return
    }
    const previous = spokenSteps.current
    spokenSteps.current = steps
    const { state: snapshot, moveSpeed } = useGameStore.getState()
    if (snapshot === null || previous === null || steps <= previous) {
      // 换局、悔棋、复盘回到开局 —— 上一手的话没说完也别说了，
      // 免得念的是另一盘棋的局面
      silence()
      tacticPly.current = 0
      return
    }
    const delay = MOVE_MS[moveSpeed]
    landAt.current = Date.now() + delay
    const speech = snapshot.status.over ? null : snapshot.last_move
    const timer = window.setTimeout(() => announce(speech, snapshot.status, steps), delay)
    return () => window.clearTimeout(timer)
  }, [steps, page])

  /**
   * 认输与超时这两种终局**没有走子**，上面那个 effect 不会触发。
   *
   * ⚠️ 这里刻意**不返回清理函数**。返回的话，终局之后任何一次快照刷新都会把
   * 这个待播的定时器取消，而重进这个 effect 时去重键已经记下了 —— 终局词就
   * 永远不出声。重复调用由 speech.ts 里的去重挡住。
   */
  useEffect(() => {
    if (page !== 'play' || state === null || !state.status.over) return
    const wait = Math.max(0, landAt.current - Date.now())
    window.setTimeout(() => announce(state.last_move, state.status, steps), wait)
  }, [state, steps, page])

  /**
   * 战法名要等讲解算完才到，所以单独一条，排在当场那几句后面。
   *
   * 只念**当前这一手**的：深度分析会把旧手数的讲解补回来，那些不该出声。
   */
  const notes = useGameStore((s) => s.notes)
  useEffect(() => {
    if (page !== 'play') return
    const { state: snapshot } = useGameStore.getState()
    if (snapshot === null) return
    const ply = snapshot.history.length
    const note = notes[ply]
    if (note === undefined || tacticPly.current === ply) return
    tacticPly.current = ply
    const wait = Math.max(0, landAt.current - Date.now())
    const timer = window.setTimeout(() => announceTactic(note, ply), wait)
    return () => window.clearTimeout(timer)
  }, [notes, page])

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
    if (state === null || state.status.over || thinking) return
    // 复盘时盘面停在过去，此刻的「走子方」是历史，不是轮到谁走
    if (state.cursor !== state.history.length) return

    if (mode === 'auto') {
      // 机机对战：暂停时两个 AI 都停手。
      // 「单步」不走这里 —— 它自己叫一次 `enginePlay`，且会先把连播关掉。
      if (!autoPlaying) return
      const wait = AUTO_PACE_MS - (Date.now() - lastMoveAt.current)
      if (wait > 0) {
        // 用定时器等剩下的时间，而不是在 store 里 sleep ——
        // 这样用户中途按暂停，cleanup 能立刻把这一步取消掉
        const timer = window.setTimeout(() => void enginePlay(), wait)
        return () => window.clearTimeout(timer)
      }
    } else if (mode === 'engine') {
      // 人机对战：只在轮到引擎那一方时替它走
      if (state.side === playerColor) return
    } else {
      // 双人同机：没有引擎的事
      return
    }

    void enginePlay()
  }, [page, mode, autoPlaying, state, playerColor, thinking, enginePlay])

  // 终局要分两种走法：普通模式跳到分析页，机机对战**留在原地开下一局**。
  // 两者都只能认「刚才还不是终局」这一个瞬间。
  const over = state?.status.over ?? false
  const wasOver = useRef(false)
  const finishAutoGame = useGameStore((s) => s.finishAutoGame)
  const coachNote = useGameStore((s) => s.coachNote)

  /**
   * 终局后**还留在对局页**，等出场特效播完再跳分析页。
   *
   * # 为什么不能立刻跳
   *
   * 特效（「绝杀」那块牌匾）是在**讲解到达之后**才开始播的，而讲解要跑一次搜索、
   * 比走子晚一两秒。立刻跳等于把整段特效吞掉 —— 而那正是这一局最该看的一下。
   *
   * # 等待分两段
   *
   * 1. 等讲解到：`coachNote.ply` 对上当前手数就说明是这一手的讲解。认输、超时
   *    没有讲解，所以这段要有上限，不能死等。
   * 2. 等特效播完：`REVEAL_MS`，与 `TacticReveal` 里那段动画同一个数（从那边导出）。
   */
  const [holdEnd, setHoldEnd] = useState(false)
  useEffect(() => {
    const justEnded = over && !wasOver.current
    wasOver.current = over
    if (!over) {
      setHoldEnd(false)
      return
    }
    if (mode === 'auto') {
      // 机机对战是**连播**的：跳走就等于停下来，与「别停」冲突。
      // 所以留在这儿记一笔战绩，倒计时结束后自动开下一局。
      if (justEnded) finishAutoGame()
      return
    }
    if (justEnded) setHoldEnd(true)
  }, [over, mode, finishAutoGame])

  // ⚠️ 依赖里放的是**手数**而不是整个 `state`：快照刷新（结算、重拉局面）会换掉
  // `state` 的身份，那样每来一次都会把待跳的定时器重置一遍 —— 等待被无限拉长，
  // 而且不会报错。
  const endPly = state?.history.length ?? 0
  const endKind = state?.status.over === true ? state.status.kind : null
  /**
   * 这个终局是**走出来的**吗？
   *
   * 只有将死 / 困毙 / 和棋是棋盘状态判出来的，也只有它们会有讲解和出场特效。
   * 认输、超时是会话层记下的，棋盘上没有那一步 —— 见 `SETTLE_MS` 的说明。
   */
  const boardEnd = endKind === 'checkmate' || endKind === 'stalemate' || endKind === 'draw'

  useEffect(() => {
    if (!holdEnd || mode === 'auto') return
    const noteReady = coachNote !== null && coachNote.ply === endPly
    const wait = boardEnd
      ? noteReady
        ? REVEAL_MS + AFTER_REVEAL_MS
        : NOTE_WAIT_MS
      : SETTLE_MS
    const timer = window.setTimeout(() => {
      setHoldEnd(false)
      navigate('analysis')
    }, wait)
    return () => window.clearTimeout(timer)
  }, [holdEnd, coachNote, boardEnd, endPly, mode])

  // 机机对战：终局且还在连播 → 排下一局（20 秒后）。
  // 单独一段是因为「暂停后按继续」也要能重新排上，而不只是终局那一刻。
  const nextGameAt = useGameStore((s) => s.nextGameAt)
  const armNextGame = useGameStore((s) => s.armNextGame)
  useEffect(() => {
    if (page !== 'play' || mode !== 'auto' || !autoPlaying || !over) return
    if (nextGameAt !== null) return
    armNextGame()
  }, [page, mode, autoPlaying, over, nextGameAt, armNextGame])

  // 到点了就开下一局。
  // 存的是**时刻**而不是倒计时秒数，所以暂停期间它自然地停住（这段 effect 被
  // cleanup 掉了），继续的时候按剩下的时间接着算。
  const startNextGame = useGameStore((s) => s.startNextGame)
  useEffect(() => {
    if (page !== 'play' || mode !== 'auto' || !autoPlaying || nextGameAt === null) return
    const remaining = nextGameAt - Date.now()
    if (remaining <= 0) {
      void startNextGame()
      return
    }
    const timer = window.setTimeout(() => void startNextGame(), remaining)
    return () => window.clearTimeout(timer)
  }, [page, mode, autoPlaying, nextGameAt, startNextGame])

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
