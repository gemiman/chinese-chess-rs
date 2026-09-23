//! 开局定式匹配。
//!
//! # 两种匹配形态
//!
//! | 形态 | 条件 | 用途 |
//! |---|---|---|
//! | **完全命中** | 定式序列是当前对局着法序列的前缀 | 「这盘棋走成了中炮对屏风马」 |
//! | **进行中命中** | 当前着法序列是定式序列的前缀 | 「正在进入中炮对屏风马」 |
//!
//! 只有完全命中才谈得上「走成了某个开局」；进行中命中用于**提前**给用户反馈，
//! 这在教学场景里价值很高 —— 用户走了两步就想知道自己下的是什么体系。
//!
//! 匹配取**最长**前缀，避免短定式（如「中炮」只要一步）盖住长定式
//! （如「中炮对屏风马」要八步）。

use crate::knowledge::{KnowledgeBase, OpeningDef};

/// 一次开局匹配的结果。
#[derive(Clone, Debug)]
pub struct OpeningMatch<'a> {
    /// 命中的定式。
    pub opening: &'a OpeningDef,
    /// 该定式的着法序列是否**已全部走完**。
    pub fully_played: bool,
    /// 当前已匹配到的步数。
    pub matched_plies: usize,
}

/// 按着法序列匹配开局定式。
///
/// `history_iccs` 是当前对局从第 1 着起的 ICCS 着法串。
pub fn match_opening<'a>(
    kb: &'a KnowledgeBase,
    history_iccs: &[String],
) -> Option<OpeningMatch<'a>> {
    if history_iccs.is_empty() {
        return None;
    }

    let mut best: Option<OpeningMatch<'a>> = None;

    for opening in &kb.openings {
        let seq = &opening.sequence_iccs;
        if seq.is_empty() {
            continue;
        }

        // 完全命中：定式是历史的前缀
        if history_iccs.len() >= seq.len()
            && history_iccs[..seq.len()]
                .iter()
                .zip(seq.iter())
                .all(|(a, b)| a == b)
        {
            let candidate = OpeningMatch {
                opening,
                fully_played: true,
                matched_plies: seq.len(),
            };
            if better(&candidate, best.as_ref()) {
                best = Some(candidate);
            }
            continue;
        }

        // 进行中命中：历史是定式的前缀（至少走了 2 步才提示，避免第 1 步就下结论）
        if history_iccs.len() >= 2
            && history_iccs.len() < seq.len()
            && history_iccs.iter().zip(seq.iter()).all(|(a, b)| a == b)
        {
            let candidate = OpeningMatch {
                opening,
                fully_played: false,
                matched_plies: history_iccs.len(),
            };
            if better(&candidate, best.as_ref()) {
                best = Some(candidate);
            }
        }
    }

    best
}

/// 候选是否优于当前最优：先看是否完全命中，再比匹配步数。
fn better(candidate: &OpeningMatch<'_>, current: Option<&OpeningMatch<'_>>) -> bool {
    match current {
        None => true,
        Some(best) => match (candidate.fully_played, best.fully_played) {
            (true, false) => true,
            (false, true) => false,
            _ => candidate.matched_plies > best.matched_plies,
        },
    }
}

/// 生成开场白的战略意图文案。
///
/// 红黑双方各有侧重 —— 同一条定式对两边的意义不同。
pub fn idea_for(opening: &OpeningDef, side: xq_core::Color) -> &str {
    let (preferred, fallback) = match side {
        xq_core::Color::Red => (&opening.idea_red, &opening.idea_black),
        xq_core::Color::Black => (&opening.idea_black, &opening.idea_red),
    };
    if !preferred.is_empty() {
        preferred
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xq_core::Color;

    fn kb() -> &'static KnowledgeBase {
        KnowledgeBase::embedded()
    }

    #[test]
    fn empty_history_matches_nothing() {
        assert!(match_opening(kb(), &[]).is_none());
    }

    /// 走完八步的正统「中炮对屏风马」应完全命中。
    #[test]
    fn full_match_central_cannon_vs_screen_horse() {
        let seq = kb()
            .openings
            .iter()
            .find(|o| o.id == "central_cannon_vs_screen_horse")
            .expect("开局库里应有这条定式")
            .sequence_iccs
            .clone();

        let matched = match_opening(kb(), &seq).expect("应命中");
        assert_eq!(matched.opening.id, "central_cannon_vs_screen_horse");
        assert!(matched.fully_played);
        assert_eq!(matched.matched_plies, seq.len());
    }

    /// 只走了前两步时，应给「进行中」的提示，而不是完全不匹配。
    #[test]
    fn partial_match_reports_in_progress() {
        let seq = kb()
            .openings
            .iter()
            .find(|o| o.id == "central_cannon_vs_screen_horse")
            .unwrap()
            .sequence_iccs
            .clone();

        let partial = &seq[..2];
        let matched = match_opening(kb(), partial).expect("两步也应能给出进行中的提示");
        assert!(!matched.fully_played);
        assert_eq!(matched.matched_plies, 2);
    }

    /// 最长匹配优先：不能因为「中炮」只要一步就盖住八步的完整定式。
    #[test]
    fn longest_match_wins() {
        let seq = kb()
            .openings
            .iter()
            .find(|o| o.id == "central_cannon_vs_screen_horse")
            .unwrap()
            .sequence_iccs
            .clone();
        let matched = match_opening(kb(), &seq).unwrap();
        assert!(
            matched.matched_plies >= 4,
            "应命中更长的定式而不是「中炮」，实际匹配 {} 步",
            matched.matched_plies
        );
    }

    /// 一步就下结论太武断。
    #[test]
    fn single_ply_does_not_claim_in_progress() {
        let seq = kb()
            .openings
            .iter()
            .find(|o| o.id == "central_cannon_vs_screen_horse")
            .unwrap()
            .sequence_iccs
            .clone();
        let matched = match_opening(kb(), &seq[..1]);
        // 单步可能因为「中炮」这类一步定式而完全命中；但绝不能是「进行中」的标记
        if let Some(m) = matched {
            assert_ne!(
                (!m.fully_played, m.matched_plies),
                (true, 1),
                "单步不应报「进行中」"
            );
        }
    }

    #[test]
    fn unmatched_sequence_returns_none() {
        let bogus = vec!["a0a1".to_string(), "a9a8".to_string()];
        assert!(match_opening(kb(), &bogus).is_none());
    }

    #[test]
    fn idea_differs_by_side() {
        let opening = &kb().openings[0];
        let red = idea_for(opening, Color::Red);
        let black = idea_for(opening, Color::Black);
        assert!(!red.is_empty());
        // 同一定式对两边的意义不同（若素材里两边文案相同，这条断言会提醒我们）
        assert_ne!(red, black, "红黑双方的战略意图不应是同一句话");
    }
}
