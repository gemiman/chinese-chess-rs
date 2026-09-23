//! 重复局面检测与长将裁决。
//!
//! # 为什么是简化裁决
//!
//! 中国象棋竞赛规则对循环局面（长将、长捉、长兑、一将一杀…）有数十页的裁决表，
//! 完整实现属于**裁判级别**的规则引擎。本项目采用**可实现、可解释的简化裁决表**，
//! 并在 UI 中明确展示裁决理由 —— 这与主流在线象棋平台（含 JJ 象棋）的做法一致。
//!
//! 简化不等于随意：本模块给出的判定是**确定性、可测试**的。
//!
//! # 已实现 / 未实现
//!
//! | 情形 | 裁决 | 状态 |
//! |---|---|---|
//! | 一方持续将军 | 该方判负 | ✅ 已实现 |
//! | 双方均持续将军 | 判和 | ✅ 已实现 |
//! | 一方长捉、另一方长将 | 长将方判负 | ⚠️ 由「长将」分支覆盖 |
//! | 一方长捉 | 该方判负 | ❌ **未实现**，暂按判和处理 |
//! | 其余（双方闲着、一将一杀等） | 判和 | ✅ 已实现 |
//!
//! 「长捉」未实现的原因：它依赖「根」（保护关系）与交换价值的计算模型，
//! 而这套模型与 `xq-coach` 的战术识别是同一套（见
//! [docs/05](../../../docs/05-战法讲解引擎设计.md)）。为避免在规则内核里
//! **重复实现一份**、并造成两处口径不一致，本模块暂按**保守判和**处理 ——
//! 宁可漏判（让对局继续），也不愿误判（把不该判负的一方判负）。
//! 该能力将在 M2 与战术识别一并落地。
//!
//! # 哈希碰撞
//!
//! 用 Zobrist 哈希而非逐格比较是工程折衷。单局数百步的规模下，64 位哈希的
//! 碰撞概率约为 `n² / 2^65`（n = 局面数），量级 `1e-15`，实际可忽略。
//!
//! > 设计文档 §6.2 建议「哈希命中后再做一次逐格比对」作为双重校验。本实现
//! > **未采纳**：`history` 只存哈希不存棋盘，逐格比对需要回放整段历史
//! > （make/unmake 全程），代价与复杂度都远超收益。若将来需要绝对严谨，
//! > 正确做法是把 Zobrist 扩成双 64 位键，而不是回放比对。

use crate::color::Color;
use crate::position::Position;

/// 触发三次重复后的裁决结果。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RepetitionVerdict {
    /// 一方持续将军，该方判负。
    LongCheck { loser: Color },
    /// 双方均持续将军，判和。
    BothLongCheck,
    /// 判和。包含双方均为闲着，以及尚未实现判定的「长捉」情形。
    Draw,
}

impl RepetitionVerdict {
    /// 面向用户的裁决理由。
    pub const fn description(self) -> &'static str {
        match self {
            RepetitionVerdict::LongCheck { .. } => "一方长将，判长将方负",
            RepetitionVerdict::BothLongCheck => "双方长将，判和",
            RepetitionVerdict::Draw => "循环局面且无长将，判和",
        }
    }

    /// 该裁决是否已分出胜负。
    pub const fn loser(self) -> Option<Color> {
        match self {
            RepetitionVerdict::LongCheck { loser } => Some(loser),
            _ => None,
        }
    }
}

/// 当前局面在历史中出现过几次（含当前这一次）。
pub fn occurrences(pos: &Position) -> usize {
    let h = pos.hash();
    pos.history().iter().filter(|&&x| x == h).count()
}

/// 当前局面是否第 3 次（或更多次）出现。
pub fn is_threefold(pos: &Position) -> bool {
    occurrences(pos) >= 3
}

/// 对当前局面给出裁决建议；未触发三次重复时返回 `None`。
///
/// 判定逻辑：取「当前局面首次出现」到「当前」之间的着法序列作为循环区间，
/// 按走子方分组统计是否**每一步都将军**。
pub fn adjudicate(pos: &Position) -> Option<RepetitionVerdict> {
    if !is_threefold(pos) {
        return None;
    }

    let history = pos.history();
    let current = history.len() - 1;
    let first = history.iter().position(|&x| x == pos.hash())?;

    // history[k] 是走完 k 步后的局面；带来该局面的着法是 move_stack[k-1]。
    // 因此「从局面 first 到局面 current」之间的着法区间是 move_stack[first..current]。
    let cycle = pos.move_stack().get(first..current)?;
    if cycle.is_empty() {
        return None;
    }

    let mut red_moves = 0usize;
    let mut red_checks = 0usize;
    let mut black_moves = 0usize;
    let mut black_checks = 0usize;

    for record in cycle {
        match record.mover {
            Color::Red => {
                red_moves += 1;
                if record.gave_check {
                    red_checks += 1;
                }
            }
            Color::Black => {
                black_moves += 1;
                if record.gave_check {
                    black_checks += 1;
                }
            }
        }
    }

    // 必须要求「该方确实走过子」，否则 0 步会因空集合的真值判断而被误判为长将。
    let red_all_check = red_moves > 0 && red_checks == red_moves;
    let black_all_check = black_moves > 0 && black_checks == black_moves;

    Some(match (red_all_check, black_all_check) {
        (true, true) => RepetitionVerdict::BothLongCheck,
        (true, false) => RepetitionVerdict::LongCheck { loser: Color::Red },
        (false, true) => RepetitionVerdict::LongCheck {
            loser: Color::Black,
        },
        (false, false) => RepetitionVerdict::Draw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mv::Move;
    use crate::square::from_iccs;

    fn mv(iccs: &str) -> Move {
        Move::new(
            from_iccs(&iccs[0..2]).expect("起点坐标非法"),
            from_iccs(&iccs[2..4]).expect("终点坐标非法"),
        )
    }

    fn play(pos: &mut Position, cycle: &[&str], times: usize) {
        for _ in 0..times {
            for iccs in cycle {
                pos.make_move(mv(iccs))
                    .unwrap_or_else(|e| panic!("着法 {iccs} 应合法: {e}"));
            }
        }
    }

    /// 双方各自来回平移车 —— 无长将，判和。
    ///
    /// 局面：红帅 `d0` + 红车 `a0`；黑将 `e9` + 黑车 `i9`，两王不同列故无照面。
    #[test]
    fn quiet_repetition_is_draw() {
        let mut pos = crate::fen::from_fen("4k3r/9/9/9/9/9/9/9/9/R2K5 w - - 0 1").unwrap();

        // 4 个半步一个循环，正好回到初始局面
        let cycle = ["a0a1", "i9i8", "a1a0", "i8i9"];
        assert_eq!(occurrences(&pos), 1);
        play(&mut pos, &cycle, 1);
        assert_eq!(occurrences(&pos), 2, "一圈之后应第 2 次出现");
        assert_eq!(adjudicate(&pos), None, "两次重复还不应触发裁决");
        play(&mut pos, &cycle, 1);
        assert_eq!(occurrences(&pos), 3, "两圈之后应第 3 次出现");

        assert!(
            !pos.move_stack().iter().any(|r| r.gave_check),
            "本循环不应有将军"
        );
        assert_eq!(adjudicate(&pos), Some(RepetitionVerdict::Draw));
    }

    /// 长将：红车在 d8/e8 之间来回，**每一步都将军**；黑将被迫在 d9/e9 之间躲。
    ///
    /// 局面设计（每一步黑将都只有唯一合法着法，循环是被强制的）：
    /// - 红帅 `e0`、红兵 `e5` —— 兵永久封住 e 列，避免两王照面；
    /// - 红车 `a8` —— 沿第 8 行保护 `d8`/`e8`，使黑将不能吃车；
    /// - 红车 `f1` —— 封住 `f9`，把黑将的退路限制在 d9/e9；
    /// - 红车 `d8` —— 将军的主角，在 d8 ↔ e8 摆动。
    #[test]
    fn long_check_loses() {
        let mut pos = crate::fen::from_fen("3k5/R2R5/9/9/4P4/9/9/9/5R3/4K4 b - - 0 1").unwrap();

        // 一个循环 4 个半步：黑将躲 → 红车将军 → 黑将回 → 红车再将军
        let cycle = ["d9e9", "d8e8", "e9d9", "e8d8"];
        play(&mut pos, &cycle, 2);

        assert!(is_threefold(&pos), "应已三次重复");

        // 循环内红方两着全部将军，黑方两着全部不将军 → 长将方是红方
        let checks: Vec<(Color, bool)> = pos
            .move_stack()
            .iter()
            .map(|r| (r.mover, r.gave_check))
            .collect();
        assert!(
            checks.iter().all(|&(c, g)| c == Color::Black || g),
            "红方在循环内应每步都将军: {checks:?}"
        );
        assert!(
            checks.iter().all(|&(c, g)| c == Color::Red || !g),
            "黑方在循环内不应将军: {checks:?}"
        );

        assert_eq!(
            adjudicate(&pos),
            Some(RepetitionVerdict::LongCheck { loser: Color::Red })
        );
    }

    /// 未达阈值时不裁决。
    #[test]
    fn below_threshold_returns_none() {
        let pos = Position::startpos();
        assert_eq!(occurrences(&pos), 1);
        assert!(!is_threefold(&pos));
        assert_eq!(adjudicate(&pos), None);
    }
}
