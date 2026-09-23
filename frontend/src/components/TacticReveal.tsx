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

/** 出场动画时长（毫秒），与 CSS 里的 2500ms 对齐。 */
const REVEAL_MS = 2500

export function TacticReveal({ note }: { note: CoachNote | null }) {
  const [shown, setShown] = useState<{ name: string; id: number } | null>(null)
  // 用「第几手 + 战法名」当键，同一步重复渲染不会重播
  const played = useRef<string | null>(null)

  useEffect(() => {
    const name = notableTactic(note)
    const id = note === null || name === null ? null : `${note.ply}:${name}`
    if (name === null || id === played.current) return
    played.current = id
    setShown({ name, id: Date.now() })
    const timer = window.setTimeout(() => setShown(null), REVEAL_MS)
    return () => window.clearTimeout(timer)
  }, [note])

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
