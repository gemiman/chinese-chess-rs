//! 模板渲染与降级链。
//!
//! # 红线：绝不允许 `{xxx}` 原样出现在用户眼前
//!
//! 渲染前做**严格校验**：模板里出现的每个占位符都必须在上下文中存在；
//! `required_always` 里的占位符还必须非空。任一不满足即**放弃该模板**，
//! 沿降级链往下走。这是被测试锁死的红线（见 `placeholder_red_line`）。
//!
//! # 降级链
//!
//! ```text
//! ① specific     最高置信度战术的专用模板
//! ② category     其余命中战术的模板
//! ③ level_generic 按评价等级的通用模板（tpl.generic.*）
//! ④ fallback     兜底模板（仅输出记谱）
//! ```
//!
//! **命中第 ④ 级不计入覆盖率**（docs/05 §5.3）—— 所以「覆盖率 ≥ 95%」是一条
//! 真实的工作量要求，不是随口一提的指标。

use std::collections::HashMap;

use crate::knowledge::{KnowledgeBase, TemplateDef};
use crate::note::{MoveLevel, TacticTag, Verbosity};

/// 渲染上下文：**全部**允许的占位符都必须在这里有值。
///
/// 设计上刻意不留 `Option` —— 缺失的占位符用语义合理的默认文本填充
/// （例如没有更优着法时 `best_reply_hint` 为「另寻它法」），
/// 这样模板永远能渲染出通顺的句子，而不是在渲染层做分支判断。
#[derive(Clone, Debug, Default)]
pub struct RenderContext {
    /// `{side}` 走子方，值为「红」或「黑」。
    pub side: String,
    /// `{notation}` 中文记谱。
    pub notation: String,
    /// `{piece_name}` 走子的棋子名。
    pub piece_name: String,
    /// `{captured_piece}` 被吃棋子名。
    pub captured_piece: String,
    /// `{best_move_notation}` 引擎最优着法的记谱。
    pub best_move_notation: String,
    /// `{score_loss}` 分差。
    pub score_loss: String,
    /// `{level_text}` 等级文案。
    pub level_text: String,
    /// `{opening_name}` 开局名。
    pub opening_name: String,
    /// `{tactic_names}` 战术名列表。
    pub tactic_names: String,
    /// `{best_reply_hint}` 最优应法描述。
    pub best_reply_hint: String,
    /// `{threat_targets}` 受威胁的棋子。
    pub threat_targets: String,
    /// `{defended_piece}` 被保护的棋子。
    pub defended_piece: String,
}

/// 允许的占位符名（与 `coach-templates.json` 的 `placeholders.allowed` 一致）。
pub const ALLOWED_PLACEHOLDERS: [&str; 12] = [
    "side",
    "notation",
    "piece_name",
    "captured_piece",
    "best_move_notation",
    "score_loss",
    "level_text",
    "opening_name",
    "tactic_names",
    "best_reply_hint",
    "threat_targets",
    "defended_piece",
];

impl RenderContext {
    /// 转成占位符查询表。
    pub fn to_map(&self) -> HashMap<&'static str, String> {
        let mut map = HashMap::with_capacity(ALLOWED_PLACEHOLDERS.len());
        map.insert("side", self.side.clone());
        map.insert("notation", self.notation.clone());
        map.insert("piece_name", self.piece_name.clone());
        map.insert("captured_piece", self.captured_piece.clone());
        map.insert("best_move_notation", self.best_move_notation.clone());
        map.insert("score_loss", self.score_loss.clone());
        map.insert("level_text", self.level_text.clone());
        map.insert("opening_name", self.opening_name.clone());
        map.insert("tactic_names", self.tactic_names.clone());
        map.insert("best_reply_hint", self.best_reply_hint.clone());
        map.insert("threat_targets", self.threat_targets.clone());
        map.insert("defended_piece", self.defended_piece.clone());
        map
    }
}

/// 渲染结果。
#[derive(Clone, Debug)]
pub struct Rendered {
    /// 实际使用的模板 id。
    pub template_id: String,
    /// 一句话结论。
    pub headline: String,
    /// 详细说明。
    pub detail: String,
    /// 是否走了兜底模板（兜底不计入覆盖率）。
    pub used_fallback: bool,
}

/// 沿降级链渲染。
pub fn render(
    kb: &KnowledgeBase,
    ctx: &RenderContext,
    level: MoveLevel,
    tactics: &[TacticTag],
    verbosity: Verbosity,
) -> Rendered {
    let map = ctx.to_map();
    let required = &kb.placeholders.required_always;

    // ① + ②：命中的战术模板。`tactics` 已按置信度降序，故首个即「专用模板」，
    //         其余为「同类战术模板」—— 对应降级链的前两级。
    for tag in tactics {
        let Some(def) = kb.tactic(&tag.id) else {
            continue;
        };
        let Some(tpl) = kb.template(&def.explanation_template) else {
            continue;
        };
        if let Some((headline, detail)) = try_render(tpl, &map, required, verbosity) {
            return Rendered {
                template_id: tpl.id.clone(),
                headline,
                detail,
                used_fallback: false,
            };
        }
    }

    // ③ 按评价等级的通用模板
    if let Some(tpl) = kb.template(level.generic_template_id())
        && let Some((headline, detail)) = try_render(tpl, &map, required, verbosity)
    {
        return Rendered {
            template_id: tpl.id.clone(),
            headline,
            detail,
            used_fallback: false,
        };
    }

    // ④ 兜底模板
    let fb = kb.fallback_template();
    match try_render(fb, &map, required, verbosity) {
        Some((headline, detail)) => Rendered {
            template_id: fb.id.clone(),
            headline,
            detail,
            used_fallback: true,
        },
        None => {
            // 连兜底模板都渲染不出（例如它引用了未知占位符）—— 这是素材被改坏的信号。
            // 这里**不再沿链下降**，而是就地拼一句最简单的，保证 analyze 永不失败。
            Rendered {
                template_id: "builtin.last_resort".to_string(),
                headline: format!("{}方 {}。", ctx.side, ctx.notation),
                detail: String::new(),
                used_fallback: true,
            }
        }
    }
}

/// 尝试渲染一个模板。任一占位符无法满足即返回 `None`。
fn try_render(
    tpl: &TemplateDef,
    map: &HashMap<&'static str, String>,
    required: &[String],
    verbosity: Verbosity,
) -> Option<(String, String)> {
    let headline = substitute(&tpl.headline, map, required)?;
    let detail = substitute(tpl.detail.pick(verbosity), map, required)?;
    Some((headline, detail))
}

/// 替换占位符。任何无法解析的占位符都会导致失败。
fn substitute(
    text: &str,
    map: &HashMap<&'static str, String>,
    required: &[String],
) -> Option<String> {
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;

    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('}')?; // 有 `{` 无 `}` —— 模板本身有问题
        let name = &after[..end];

        let value = map.get(name)?; // 未知占位符 → 放弃该模板
        if value.is_empty() && required.iter().any(|r| r == name) {
            return None; // 必需占位符为空 → 放弃该模板
        }
        out.push_str(value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);

    // 收尾断言：不允许有任何残留的 `}`（说明模板里有孤立的 `}`）
    if out.contains('}') {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::TacticCategory;

    fn full_context() -> RenderContext {
        RenderContext {
            side: "红".into(),
            notation: "炮二平五".into(),
            piece_name: "炮".into(),
            captured_piece: "卒".into(),
            best_move_notation: "马八进七".into(),
            score_loss: "0".into(),
            level_text: "最佳着法".into(),
            opening_name: "中炮对屏风马".into(),
            tactic_names: "中炮".into(),
            best_reply_hint: "起马保卒".into(),
            threat_targets: "黑马".into(),
            defended_piece: "红兵".into(),
        }
    }

    #[test]
    fn context_covers_every_allowed_placeholder() {
        let map = full_context().to_map();
        for name in ALLOWED_PLACEHOLDERS {
            assert!(map.contains_key(name), "上下文缺少占位符 {name}");
        }
        assert_eq!(map.len(), ALLOWED_PLACEHOLDERS.len());
    }

    /// **红线测试**：遍历模板库全部模板，渲染后不得残留 `{` 或 `}`。
    ///
    /// 这是 docs/05 §5.2 的强制要求 ——「绝不允许出现 `{xxx}` 原样输出到用户眼前」。
    #[test]
    fn placeholder_red_line() {
        let kb = KnowledgeBase::embedded();
        let ctx = full_context();
        let map = ctx.to_map();

        for tpl in &kb.templates {
            for verbosity in [Verbosity::Concise, Verbosity::Standard, Verbosity::Verbose] {
                // 有必需占位符缺失时 try_render 会返回 None，那也是合法结果；
                // 这里检查的是**渲染出来的时候**不能有残留花括号。
                if let Some((headline, detail)) = try_render(tpl, &map, &[], verbosity) {
                    assert!(
                        !headline.contains('{') && !headline.contains('}'),
                        "模板 {} 的 headline 渲染后残留占位符：{headline}",
                        tpl.id
                    );
                    assert!(
                        !detail.contains('{') && !detail.contains('}'),
                        "模板 {} 的 detail 渲染后残留占位符：{detail}",
                        tpl.id
                    );
                }
            }
        }
    }

    /// 用空上下文（缺必需项）渲染时，应当返回 None 而不是渲染出空占位符。
    #[test]
    fn empty_required_placeholder_aborts_render() {
        let tpl = TemplateDef {
            id: "t".into(),
            used_by: vec![],
            headline: "{side}方走 {notation}。".into(),
            detail: crate::knowledge::TemplateDetail {
                concise: "c".into(),
                standard: "s".into(),
                verbose: "v".into(),
            },
            is_fallback: false,
        };
        let mut map = HashMap::new();
        map.insert("side", "红".to_string());
        map.insert("notation", String::new());

        let required = vec!["side".to_string(), "notation".to_string()];
        assert!(
            try_render(&tpl, &map, &required, Verbosity::Concise).is_none(),
            "必需的 notation 为空时必须放弃该模板"
        );
    }

    #[test]
    fn unknown_placeholder_aborts_render() {
        let tpl = TemplateDef {
            id: "t".into(),
            used_by: vec![],
            headline: "{不存在的占位符}".into(),
            detail: crate::knowledge::TemplateDetail {
                concise: "c".into(),
                standard: "s".into(),
                verbose: "v".into(),
            },
            is_fallback: false,
        };
        assert!(try_render(&tpl, &full_context().to_map(), &[], Verbosity::Concise).is_none());
    }

    #[test]
    fn unmatched_brace_aborts_render() {
        let tpl = TemplateDef {
            id: "t".into(),
            used_by: vec![],
            headline: "{side 少了右括号".into(),
            detail: crate::knowledge::TemplateDetail {
                concise: "c".into(),
                standard: "s".into(),
                verbose: "v".into(),
            },
            is_fallback: false,
        };
        assert!(try_render(&tpl, &full_context().to_map(), &[], Verbosity::Concise).is_none());
    }

    /// 无战术命中时应落到「按等级」的通用模板，且不算兜底。
    #[test]
    fn falls_back_to_level_generic_without_tactics() {
        let kb = KnowledgeBase::embedded();
        let rendered = render(
            kb,
            &full_context(),
            MoveLevel::Best,
            &[],
            Verbosity::Standard,
        );
        assert_eq!(rendered.template_id, "tpl.generic.best");
        assert!(!rendered.used_fallback, "通用等级模板不算兜底");
        assert!(rendered.headline.contains("炮二平五"));
    }

    /// 有战术命中时优先用专用模板。
    #[test]
    fn uses_specific_template_when_tactic_matched() {
        let kb = KnowledgeBase::embedded();
        let tactics = vec![TacticTag {
            id: "check".into(),
            name: "将军".into(),
            category: TacticCategory::Structure,
            confidence: 1.0,
        }];
        let rendered = render(
            kb,
            &full_context(),
            MoveLevel::Best,
            &tactics,
            Verbosity::Standard,
        );
        assert_eq!(rendered.template_id, "tpl.check");
        assert!(rendered.headline.contains("将军"));
        assert!(!rendered.used_fallback);
    }

    /// 引用了不存在模板的战术标签应被跳过，继续走降级链。
    #[test]
    fn unknown_tactic_id_is_skipped() {
        let kb = KnowledgeBase::embedded();
        let tactics = vec![TacticTag {
            id: "不存在的战术".into(),
            name: "?".to_string(),
            category: TacticCategory::Relation,
            confidence: 0.5,
        }];
        let rendered = render(
            kb,
            &full_context(),
            MoveLevel::Good,
            &tactics,
            Verbosity::Concise,
        );
        assert_eq!(rendered.template_id, "tpl.generic.good");
    }
}
