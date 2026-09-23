import { navigate } from '../router'
import { useGameStore } from '../store'
import {
  DIFFICULTIES,
  MOVE_SPEEDS,
  THINK_MS,
  TIME_PRESETS,
  timePresetOf,
  type Color,
  type GameMode,
} from '../types'

/**
 * 开局设置页。
 *
 * # 为什么单独一页
 *
 * 原来这些选项挤在对局页的侧栏里，和棋谱、提示、FEN 混在一起。结果是
 * **开局前**要在一堆和开局无关的东西里找选项，**开局后**又永远占着一块地方。
 * 拆开之后两件事各自干净：这里只回答「这一局怎么下」，对局页只回答「现在什么局面」。
 *
 * # 为什么「开始游戏」是这里唯一的出口
 *
 * 改选项**不会**立刻作用到正在下的这局（限时、模式都是开局时下发给 Rust 的），
 * 所以必须有一个明确的「开始」动作。把它做成唯一出口，用户就不会有
 * 「我选了怎么没生效」的困惑。
 */

const SIDE_LABEL: Record<string, string> = { red: '红方', black: '黑方' }

export function SetupPage() {
  const mode = useGameStore((s) => s.mode)
  const playerColor = useGameStore((s) => s.playerColor)
  const difficulty = useGameStore((s) => s.difficulty)
  const timePreset = useGameStore((s) => s.timePreset)
  const moveSpeed = useGameStore((s) => s.moveSpeed)
  const busy = useGameStore((s) => s.busy)
  const setMode = useGameStore((s) => s.setMode)
  const setPlayerColor = useGameStore((s) => s.setPlayerColor)
  const setDifficulty = useGameStore((s) => s.setDifficulty)
  const setTimePreset = useGameStore((s) => s.setTimePreset)
  const setMoveSpeed = useGameStore((s) => s.setMoveSpeed)
  const startGame = useGameStore((s) => s.startGame)

  const activeDifficulty = DIFFICULTIES.find((d) => d.id === difficulty)

  async function start() {
    await startGame()
    // 开局失败（例如引擎连不上）就别跳走 —— 跳过去用户就看不到错误提示了
    if (useGameStore.getState().error === null) navigate('play')
  }

  return (
    <div className="setup">
      <div className="setup__intro">
        <h2 className="setup__heading">开始一局新棋</h2>
        <p className="muted">
          选好怎么下，按下面的「开始游戏」。这些设置**开局后不能改** ——
          想换一种下法，回来重开一局。
        </p>
      </div>

      <div className="setup__grid">
        <section className="card">
          <h3 className="card__title">对局模式</h3>
          <div className="segmented segmented--wide" role="group" aria-label="对局模式">
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
          <p className="muted setup__hint">
            {mode === 'engine'
              ? '你和 Rust 引擎对下，轮流一步。'
              : '两个人共用一块棋盘，轮流点棋子走。'}
          </p>
        </section>

        {mode === 'engine' ? (
          <section className="card">
            <h3 className="card__title">我执哪一方</h3>
            <div className="segmented segmented--wide" role="group" aria-label="我执哪一方">
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
            <p className="muted setup__hint">
              执黑时棋盘会翻转过来，你的棋始终在下方。
            </p>
          </section>
        ) : null}

        {mode === 'engine' ? (
          <section className="card setup__card--wide">
            <h3 className="card__title">引擎棋力</h3>
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
              <p className="muted setup__hint">
                {activeDifficulty.subtitle} · 每步思考约 {THINK_MS[difficulty]} 毫秒
              </p>
            ) : null}
          </section>
        ) : null}

        <section className="card setup__card--wide">
          <h3 className="card__title">限时</h3>
          <div className="segmented segmented--wrap" role="group" aria-label="限时档位">
            {TIME_PRESETS.map((item) => (
              <button
                key={item.id}
                type="button"
                className={`segmented__item${
                  timePreset === item.id ? ' segmented__item--active' : ''
                }`}
                aria-pressed={timePreset === item.id}
                onClick={() => setTimePreset(item.id)}
              >
                {item.label}
              </button>
            ))}
          </div>
          <p className="muted setup__hint">{timePresetOf(timePreset).hint}</p>
        </section>

        <section className="card">
          <h3 className="card__title">走子速度</h3>
          <div className="segmented segmented--wide" role="group" aria-label="走子动画速度">
            {MOVE_SPEEDS.map((item) => (
              <button
                key={item.id}
                type="button"
                className={`segmented__item${moveSpeed === item.id ? ' segmented__item--active' : ''}`}
                aria-pressed={moveSpeed === item.id}
                onClick={() => setMoveSpeed(item.id)}
              >
                {item.label}
              </button>
            ))}
          </div>
          <p className="muted setup__hint">「慢动作」是留给复盘看棋用的。</p>
        </section>

        <section className="card">
          <h3 className="card__title">标记说明</h3>
          <div className="legend">
            <LegendItem file="fx-target-dot" text="白点 = 该处为空，可以走过去" />
            <LegendItem file="fx-capture" text="红环 = 该处有对方棋子，可以吃掉" />
            <LegendItem file="fx-select" text="金环 = 当前选中的棋子" />
            <LegendItem file="fx-last-move" text="白弧 = 上一着的起点与终点" />
          </div>
          <p className="muted setup__hint">
            每种状态都用「颜色 + 形状」双重区分 —— 只靠颜色在石色棋盘上对比度不足。
          </p>
        </section>
      </div>

      <div className="setup__start">
        <button type="button" className="btn btn--primary btn--big" onClick={() => void start()} disabled={busy}>
          {busy ? '正在摆棋…' : '开始游戏'}
        </button>
      </div>
    </div>
  )
}

function LegendItem({ file, text }: { file: string; text: string }) {
  return (
    <div className="legend__item">
      <span className="legend__swatch">
        <img src={`/effects/${file}.svg`} alt="" />
      </span>
      {text}
    </div>
  )
}
