//! 着法性质分类（`MoveNature`）。
//!
//! 本模块是 `xq-core` 提供给 [`xq-coach`] 的接口：讲解引擎需要知道「这一步是
//! 将军、吃子、垫子还是闲着」，才能选取合适的模板并给出评价。
//!
//! # 已实现 / 未实现
//!
//! | 性质 | 判定方式 | 状态 |
//! |---|---|---|
//! | 将军 | 走子后对方被将 | ✅ |
//! | 吃将军的子 | 走子前被将，且本步吃掉了某个子 | ✅ |
//! | 避将 | 走子前被将，且移动的是己方将帅 | ✅ |
//! | 垫子解将 | 走子前被将，且移动的不是将帅、也未吃子 | ✅ |
//! | 吃子 | 走子前未被将，且吃掉了某个子 | ✅ |
//! | 闲着 | 以上皆非 | ✅ |
//! | 捉 | 需要「根」与交换价值模型 | ❌ 与 `xq-coach` 的战术识别一并落地 |
//! | 兑 | 同上 | ❌ |
//!
//! 「捉 / 兑」的判定依赖保护关系（根）与子力交换价值计算，与战术识别是同一套
//! 模型。为**避免在规则内核里重复实现第二份**，本模块暂不产出这两个变体 ——
//! 调用方必须对 `Chase` / `Exchange` 做兜底处理，不能假定它们一定会出现。

use crate::mv::Move;
use crate::piece::{EMPTY, PieceKind, kind_of};
use crate::position::Position;

/// 一步棋的性质。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveNature {
    /// 将军。
    Check,
    /// 吃子（走子前未被将）。
    Capture,
    /// 捉：下一步可吃掉对方某个未被足够保护的子。
    ///
    /// > 当前版本**不会产出**该变体，见模块文档。
    Chase,
    /// 兑：与对方等价交换。
    ///
    /// > 当前版本**不会产出**该变体，见模块文档。
    Exchange,
    /// 垫子解将。
    Interpose,
    /// 避将（移动被将的将帅）。
    Escape,
    /// 吃掉将军的棋子。
    CaptureAttacker,
    /// 闲着。
    Idle,
}

impl MoveNature {
    /// 面向用户的中文名称。
    pub const fn name_zh(self) -> &'static str {
        match self {
            MoveNature::Check => "将军",
            MoveNature::Capture => "吃子",
            MoveNature::Chase => "捉",
            MoveNature::Exchange => "兑",
            MoveNature::Interpose => "垫子解将",
            MoveNature::Escape => "避将",
            MoveNature::CaptureAttacker => "吃将军的子",
            MoveNature::Idle => "闲着",
        }
    }

    /// 是否具有强制性（对方必须立即应对）。
    pub const fn is_forcing(self) -> bool {
        matches!(
            self,
            MoveNature::Check | MoveNature::CaptureAttacker | MoveNature::Escape
        )
    }
}

impl Position {
    /// 判定一步棋的性质。
    ///
    /// 判定优先级：将军 > 解将类（吃将军的子 / 避将 / 垫子）> 吃子 > 闲着。
    ///
    /// `mv` 不必是合法着法，但调用方应传入合法着法，否则结果无意义。
    pub fn nature_of(&mut self, mv: Move) -> MoveNature {
        let us = self.side_to_move();
        let was_in_check = self.is_in_check(us);
        let moving_kind = kind_of(self.piece_at(mv.from()));
        let captured = self.piece_at(mv.to());

        self.make_move_unchecked(mv);
        let gives_check = self.is_attacked(self.king_index(us.opponent()), us);
        self.unmake_move_unchecked();

        if gives_check {
            return MoveNature::Check;
        }

        if was_in_check {
            if captured != EMPTY {
                return MoveNature::CaptureAttacker;
            }
            if moving_kind == Some(PieceKind::King) {
                return MoveNature::Escape;
            }
            return MoveNature::Interpose;
        }

        if captured != EMPTY {
            return MoveNature::Capture;
        }

        MoveNature::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::mv::Move;
    use crate::position::position_from_pieces;
    use crate::square::from_iccs;

    fn mv(iccs: &str) -> Move {
        Move::new(
            from_iccs(&iccs[0..2]).unwrap(),
            from_iccs(&iccs[2..4]).unwrap(),
        )
    }

    #[test]
    fn idle_and_capture() {
        // 红帅放 d0、黑将放 e9 —— 两王不同列，避免「白脸将」把测试局面变成非法局面
        let mut pos = position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "d0"),
                (Color::Red, PieceKind::Chariot, "a0"),
                (Color::Black, PieceKind::Pawn, "a3"),
                (Color::Black, PieceKind::King, "e9"),
            ],
            Color::Red,
        )
        .unwrap();
        assert_eq!(pos.nature_of(mv("a0a1")), MoveNature::Idle);
        assert_eq!(pos.nature_of(mv("a0a3")), MoveNature::Capture);
    }

    /// 红车从 `e1` 走到 `e8`，与黑将 `e9` 同列相邻 → 将军。
    #[test]
    fn check_is_detected() {
        let mut pos = position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "d0"),
                (Color::Red, PieceKind::Chariot, "e1"),
                (Color::Black, PieceKind::King, "e9"),
                // 黑卒放在 a 列，避免干扰 e 列射线
                (Color::Black, PieceKind::Pawn, "a4"),
            ],
            Color::Red,
        )
        .unwrap();
        assert_eq!(pos.nature_of(mv("e1e8")), MoveNature::Check);
        // 走到 e7 仍然隔着 e8 空格与 e9 黑将同列 → 依然是将军
        assert_eq!(pos.nature_of(mv("e1e7")), MoveNature::Check);
        // 走到 a1 完全脱离 e 列 → 闲着
        assert_eq!(pos.nature_of(mv("e1a1")), MoveNature::Idle);
    }

    /// 被将时：走将帅 = 避将；吃掉将军的子 = CaptureAttacker；垫子 = Interpose。
    #[test]
    fn responses_to_check_are_classified() {
        // 黑车 e8 将军红帅 e0
        let mut escape = position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Black, PieceKind::Chariot, "e8"),
                (Color::Black, PieceKind::King, "f9"),
            ],
            Color::Red,
        )
        .unwrap();
        assert_eq!(escape.nature_of(mv("e0d0")), MoveNature::Escape);

        // 红车 a8 吃掉将军的黑车 e8
        let mut capture = position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Chariot, "a8"),
                (Color::Black, PieceKind::Chariot, "e8"),
                (Color::Black, PieceKind::King, "f9"),
            ],
            Color::Red,
        )
        .unwrap();
        assert_eq!(capture.nature_of(mv("a8e8")), MoveNature::CaptureAttacker);

        // 红车 a5 垫到 e5 解将
        let mut interpose = position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Chariot, "a5"),
                (Color::Black, PieceKind::Chariot, "e8"),
                (Color::Black, PieceKind::King, "f9"),
            ],
            Color::Red,
        )
        .unwrap();
        assert_eq!(interpose.nature_of(mv("a5e5")), MoveNature::Interpose);
    }

    /// 当前版本不产出 Chase / Exchange —— 调用方必须能处理这两个「永不出现」的变体。
    #[test]
    fn chase_and_exchange_are_never_produced_yet() {
        let mut pos = Position::startpos();
        let moves = pos.legal_moves();
        for mv in moves {
            let nature = pos.nature_of(mv);
            assert!(
                !matches!(nature, MoveNature::Chase | MoveNature::Exchange),
                "初始局面第 1 手不应产出 Chase/Exchange，实际 {nature:?}"
            );
        }
    }
}
