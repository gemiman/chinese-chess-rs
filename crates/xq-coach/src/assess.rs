//! 评价定级：把「分差」映射为等级。
//!
//! # 定级依据完全客观
//!
//! ```text
//! score_loss = score_best − score_played
//! ```
//!
//! 两侧都是**走子方视角**的厘兵评分，直接来自引擎的 `root_moves` ——
//! 不需要额外搜索。这是把「评分全部根着法」作为引擎核心输出的直接收益
//! （见 [docs/04 §9](../../../docs/04-AI引擎设计.md)）。
//!
//! > ⚠️ 阈值（10 / 50 / 150 / 400）是**设计初始值**，
//! > [docs/05 §9.2](../../../docs/05-战法讲解引擎设计.md) 明确要求用 200 局自对弈
//! > 的分差分布回测校准后回写。未经校准前不得对外宣称等级判定「准确」。

use xq_core::Move;

use crate::note::MoveLevel;

/// 等级阈值（厘兵）。分界点是开区间，即 `≤ 10` 为 Best、`10 < x ≤ 50` 为 Good。
pub const THRESHOLD_BEST: i32 = 10;
/// 良的上界。
pub const THRESHOLD_GOOD: i32 = 50;
/// 疑的上界。
pub const THRESHOLD_DUBIOUS: i32 = 150;
/// 劣的上界，超过即为「漏」。
pub const THRESHOLD_BLUNDER: i32 = 400;

/// 「均势局面」判定阈值。
///
/// 分差计算依赖走子前的绝对评分；当双方接近均势时，150 厘兵的「分差」实际影响有限，
/// 不应标为「明显失误」—— 因此均势局面下等级上限压到 `Dubious`。
pub const BALANCED_THRESHOLD: i32 = 100;

/// 一次评价的完整结果。
#[derive(Clone, Debug)]
pub struct Assessment {
    /// 等级。
    pub level: MoveLevel,
    /// 分差（厘兵，恒 ≥ 0）。
    pub score_loss: i32,
    /// 走子前评分（走子方视角）。
    pub score_before: i32,
    /// 实际着法的评分（走子方视角）。
    pub score_after: i32,
    /// 引擎推荐的最优着法。
    pub best_move: Option<Move>,
    /// 是否因「均势保护」而压低了等级。
    pub capped_by_balance: bool,
    /// 是否因「该着法成杀」而强制判优。
    pub forced_by_mate: bool,
    /// 是否因「错失必胜」而强制判漏。
    pub forced_by_missed_mate: bool,
}

/// 评价一步棋。
///
/// # 参数
///
/// - `root_moves`：引擎给出的**全部**根着法及其评分，需按评分降序（`xq-ai` 的输出约定）；
/// - `played`：实际走的着法；
/// - `played_mates`：该着法是否直接成杀；
/// - `best_mates`：引擎最优着法是否成杀。
pub fn assess(
    root_moves: &[(Move, i32)],
    played: Move,
    played_mates: bool,
    best_mates: bool,
) -> Assessment {
    let best = root_moves.first().copied();
    let (best_move, score_before) = match best {
        Some((mv, score)) => (Some(mv), score),
        None => (None, 0),
    };

    // 实际着法的评分。找不到（例如传入的是非法着法）时退化为「最优分减一个极大值」，
    // 使等级落到 Missed —— 非法着法本就不该出现在正常流程里。
    let score_after = root_moves
        .iter()
        .find(|(mv, _)| *mv == played)
        .map(|(_, score)| *score)
        .unwrap_or(score_before - (THRESHOLD_BLUNDER + 1));

    let score_loss = (score_before - score_after).max(0);

    let mut level = match score_loss {
        x if x <= THRESHOLD_BEST => MoveLevel::Best,
        x if x <= THRESHOLD_GOOD => MoveLevel::Good,
        x if x <= THRESHOLD_DUBIOUS => MoveLevel::Dubious,
        x if x <= THRESHOLD_BLUNDER => MoveLevel::Blunder,
        _ => MoveLevel::Missed,
    };

    // ---- 特殊规则覆盖 ----

    // ① 该着法直接成杀 → 无条件判优
    let forced_by_mate = played_mates;
    if forced_by_mate {
        level = MoveLevel::Best;
    }

    // ② 错失必胜：最优着法成杀而实际没走 → 判漏
    let forced_by_missed_mate = !forced_by_mate && best_mates;
    if forced_by_missed_mate {
        level = MoveLevel::Missed;
    }

    // ③ 均势保护：走子前接近均势时，不把「失误」说得太重
    let mut capped_by_balance = false;
    if !forced_by_mate
        && !forced_by_missed_mate
        && score_before.abs() <= BALANCED_THRESHOLD
        && level > MoveLevel::Dubious
    {
        level = MoveLevel::Dubious;
        capped_by_balance = true;
    }

    Assessment {
        level,
        score_loss,
        score_before,
        score_after,
        best_move,
        capped_by_balance,
        forced_by_mate,
        forced_by_missed_mate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(from: u8, to: u8) -> Move {
        Move::new(from, to)
    }

    /// 构造一个「最优分 = best、某着法分 = best - loss」的根着法表。
    fn roots(best: i32, loss: i32) -> Vec<(Move, i32)> {
        vec![(mv(0, 1), best), (mv(2, 3), best - loss)]
    }

    fn level_for_loss(best: i32, loss: i32) -> MoveLevel {
        assess(&roots(best, loss), mv(2, 3), false, false).level
    }

    /// 边界值逐个验证（docs/05 §9.1 要求的 9/10/11、49/50/51、149/150/151、399/400/401）。
    #[test]
    fn level_boundaries_are_exact() {
        // 用远离 0 的基准分，避免触发均势保护
        const BASE: i32 = 600;

        assert_eq!(level_for_loss(BASE, 9), MoveLevel::Best);
        assert_eq!(level_for_loss(BASE, 10), MoveLevel::Best);
        assert_eq!(level_for_loss(BASE, 11), MoveLevel::Good);

        assert_eq!(level_for_loss(BASE, 49), MoveLevel::Good);
        assert_eq!(level_for_loss(BASE, 50), MoveLevel::Good);
        assert_eq!(level_for_loss(BASE, 51), MoveLevel::Dubious);

        assert_eq!(level_for_loss(BASE, 149), MoveLevel::Dubious);
        assert_eq!(level_for_loss(BASE, 150), MoveLevel::Dubious);
        assert_eq!(level_for_loss(BASE, 151), MoveLevel::Blunder);

        assert_eq!(level_for_loss(BASE, 399), MoveLevel::Blunder);
        assert_eq!(level_for_loss(BASE, 400), MoveLevel::Blunder);
        assert_eq!(level_for_loss(BASE, 401), MoveLevel::Missed);
    }

    #[test]
    fn best_move_is_always_best_level() {
        let a = assess(&roots(600, 0), mv(0, 1), false, false);
        assert_eq!(a.level, MoveLevel::Best);
        assert_eq!(a.score_loss, 0);
        assert_eq!(a.score_after, 600);
    }

    #[test]
    fn playing_a_mate_is_best_regardless_of_score() {
        // 极端情形：引擎认为该着法分差巨大，但它其实成杀
        let a = assess(&roots(600, 900), mv(2, 3), true, false);
        assert_eq!(a.level, MoveLevel::Best);
        assert!(a.forced_by_mate);
    }

    #[test]
    fn missing_a_mate_is_missed_regardless_of_score() {
        // 最优着法是杀，实际走了别的；即便分差很小也判漏
        let a = assess(&roots(600, 5), mv(2, 3), false, true);
        assert_eq!(a.level, MoveLevel::Missed);
        assert!(a.forced_by_missed_mate);
    }

    #[test]
    fn balanced_position_caps_level_at_dubious() {
        // 均势（基准分 30）+ 巨大分差 → 不应标为「明显失误」
        let a = assess(&roots(30, 900), mv(2, 3), false, false);
        assert_eq!(a.level, MoveLevel::Dubious);
        assert!(a.capped_by_balance);
    }

    #[test]
    fn balanced_cap_does_not_upgrade_good_moves() {
        let a = assess(&roots(30, 5), mv(2, 3), false, false);
        assert_eq!(a.level, MoveLevel::Best, "均势保护只压不抬");
        assert!(!a.capped_by_balance);
    }

    #[test]
    fn empty_root_moves_is_handled_conservatively() {
        // 没有根着法评分时只能保守：走子前评分视为 0（未知），
        // 于是「均势保护」生效，等级压在 Dubious 而不是武断地判「严重漏着」。
        let a = assess(&[], mv(0, 1), false, false);
        assert_eq!(a.level, MoveLevel::Dubious);
        assert!(a.best_move.is_none());
        assert!(a.capped_by_balance);
    }

    #[test]
    fn unknown_played_move_degrades_to_missed() {
        let a = assess(&roots(600, 0), mv(9, 9), false, false);
        assert_eq!(a.level, MoveLevel::Missed);
    }

    #[test]
    fn level_ordering_is_from_best_to_worst() {
        assert!(MoveLevel::Best < MoveLevel::Good);
        assert!(MoveLevel::Good < MoveLevel::Dubious);
        assert!(MoveLevel::Dubious < MoveLevel::Blunder);
        assert!(MoveLevel::Blunder < MoveLevel::Missed);
    }

    /// 每档都必须有文字标签与图标 —— 不允许仅靠颜色传达等级。
    #[test]
    fn every_level_has_text_and_glyph() {
        for level in MoveLevel::ALL {
            assert!(!level.label().is_empty());
            assert!(!level.glyph().is_empty());
            assert!(!level.token().is_empty());
            assert!(!level.generic_template_id().is_empty());
        }
    }
}
