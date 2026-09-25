import { useEffect, useRef, useState } from 'react'

import type { CoachNote } from '../types'

/**
 * 战法名出场：压场 → 辉光 → 光束 → 牌匾 → 金字。
 *
 * # 只在「值得出场」的战法上放
 *
 * 每走一步都炸一次场，观众会烦。只挑两类：
 * **绝杀**（mate / 困毙）与**命名棋形**（卧槽马、马后炮、天地炮…）。
 * 「闲着」「吃子」这类几乎每步都有的，不出场。
 *
 * # 名字是文字不是图
 *
 * 「牌匾」只是一张底衬，名字由 CSS 渲染 —— 换「马后炮」「天地炮」不用新素材。
 * 金字靠 `background-clip: text` + 三层 `drop-shadow`：第一层零模糊是**硬描边**，
 * 专门把金字从亮橙底上拔出来，后面两层才是辉光。顺序不能反。
 */

/** 从讲解里挑出值得放出场动画的战法名；没有就返回 `null`。 */
export function notableTactic(note: CoachNote | null): string | null {
  if (!note || note.tactics.length === 0) return null
  // 太低置信度的棋形会把误报放大成一次很显眼的动画，宁缺毋滥
  const strong = note.tactics.filter((t) => t.confidence >= 0.7)
  const mate = strong.find((t) => t.id === 'mate' || t.id === 'stalemate_win')
  if (mate) return mate.name
  const formation = strong.find((t) => t.category === 'formation')
  return formation ? formation.name : null
}

/**
 * 出场动画时长（毫秒），与 CSS 里的 2500ms 对齐。
 *
 * 导出是给 `App.tsx` 用的：终局跳分析页要**等这段播完**，两处各写一个数迟早会漂。
 */
export const REVEAL_MS = 2500

export function TacticReveal({ note }: { note: CoachNote | null }) {
  const [shown, setShown] = useState<{ name: string; id: number } | null>(null)
  // 用「第几手 + 战法名」当键，同一步重复渲染不会重播
  const played = useRef<string | null>(null)

  // 什么时候**出场**：只在换了一手值得出场的战法时触发一次。
  useEffect(() => {
    const name = notableTactic(note)
    const id = note === null || name === null ? null : `${note.ply}:${name}`
    if (name === null || id === played.current) return
    played.current = id
    setShown({ name, id: Date.now() })
  }, [note])

  // 什么时候**退场**：只跟 `shown` 走，跟讲解没关系。
  //
  // ⚠️ 这段计时**绝不能**写进上面那个 effect。写进去的话，cleanup 会在 `note`
  // 一变时就把 `clearTimeout` 掉 —— 而「下一步没有值得出场的战法」恰恰是最常见的
  // 情况（精彩的杀着之后往往跟一步闲着），那条分支直接 `return`，不会重设计时器。
  // 结果就是**这一层永远卸不掉**：遮罩早已淡回透明，可容器自己还有一块不透明的
  // 深色底压着，棋盘整块变黑，而且再也回不来。
  //
  // 这个 bug 用户真的撞上了：走一步妙手、对方随手一应，棋盘就黑了。
  // 而且它**极难在测试里复现** —— 得让下一步的讲解在那 2.5 秒之内到达
  // 而且那一步恰好没有战法可讲。
  useEffect(() => {
    if (shown === null) return
    const timer = window.setTimeout(() => setShown(null), REVEAL_MS)
    return () => window.clearTimeout(timer)
  }, [shown])

  if (shown === null) return null

  return (
    // key 换掉即重挂载，动画才会从头播
    <div className="tactic" key={shown.id} aria-hidden="true">
      <div className="tactic__scrim" />
      <img className="tactic__rays" src="/effects/tactic-rays.svg" alt="" draggable={false} />
      <img className="tactic__plate" src="/effects/tactic-plate.svg" alt="" draggable={false} />
      <div className="tactic__flash" />
      <div className="tactic__name">{shown.name}</div>
    </div>
  )
}
