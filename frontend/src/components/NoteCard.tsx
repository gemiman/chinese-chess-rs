import type { CoachNote, EngineInfo, MoveLevel, TacticCategory } from '../types'

/**
 * 一条战法讲解。
 *
 * 对局页（看当前这一步）与分析页（逐手翻整局）共用同一张卡 —— 同一份数据
 * 换个地方就换个画法，是最容易两边不一致的地方，所以只留一个实现。
 */

export const LEVEL_LABEL: Record<string, string> = {
  best: '最佳着法',
  good: '不错',
  dubious: '稍有问题',
  blunder: '明显失误',
  missed: '严重漏着',
}

/** 等级图标 —— 形状各异，供色觉障碍用户区分。绝不能只用颜色。 */
export const LEVEL_GLYPH: Record<string, string> = {
  best: '★',
  good: '✓',
  dubious: '?',
  blunder: '✕',
  missed: '‼',
}

export const CATEGORY_LABEL: Record<string, string> = {
  structure: '结构层（确定）',
  relation: '关系层',
  formation: '棋形层（待校准）',
  opening: '开局',
}

export const LEVEL_ORDER: MoveLevel[] = ['best', 'good', 'dubious', 'blunder', 'missed']

export const CATEGORY_ORDER: TacticCategory[] = [
  'structure',
  'relation',
  'formation',
  'opening',
]

/** 把厘兵评分格式化成带正负号的形式。 */
export function formatScore(score: number): string {
  const sign = score > 0 ? '+' : ''
  return `${sign}${(score / 100).toFixed(2)}`
}

/** 杀棋距离的中文描述。 */
export function mateText(distance: number): string {
  return distance > 0 ? `我方 ${distance} 步内将死` : `我方 ${-distance} 步内被将死`
}

interface NoteCardProps {
  note: CoachNote | null
  /** 为生成这条讲解所做的那次搜索的元信息。 */
  info?: EngineInfo | null
  /** 正在生成。 */
  loading?: boolean
  /** 没有内容时显示什么。 */
  empty?: string
  /** 紧凑模式：分析页逐手列表里用，省掉重复的元信息。 */
  compact?: boolean
}

/**
 * 等级展示的**三重编码**（强制约束）。
 *
 * `assets/tokens/colors.md` 的 WCAG 实算结论：等级色在浅色底上对比度普遍不达标
 * （`coach.best` 只有 2.87:1）。因此等级**不允许只用颜色传达** ——
 * 本卡片同时给出：颜色底（徽标背景）+ 图标形状（★ ✓ ? ✕ ‼）+ 文字标签。
 */
export function NoteCard({ note, info, loading, empty, compact }: NoteCardProps) {
  if (note === null) {
    return (
      <div className="note note--empty">
        <p className="muted" style={{ margin: 0 }}>
          {loading ? '正在生成讲解…' : (empty ?? '走一步棋，这里会给出这步的战法名称与评价。')}
        </p>
      </div>
    )
  }

  return (
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

      {note.opening_name !== null ? <p className="note__meta">开局：{note.opening_name}</p> : null}

      {note.suggestion !== null && !compact ? (
        <p className="note__suggestion">{note.suggestion}</p>
      ) : null}

      {note.used_fallback ? (
        <p className="note__meta">
          （这条讲解走了兜底模板 —— 说明该着法暂时没有匹配到合适的战术模板）
        </p>
      ) : null}

      {info !== null && info !== undefined && !compact ? (
        <p className="note__meta">
          评价依据：引擎搜索 {info.depth} 层 · {info.nodes.toLocaleString()} 节点 ·{' '}
          {info.think_ms} ms（分差单位为厘兵，100 = 一个兵）
        </p>
      ) : null}
    </div>
  )
}
