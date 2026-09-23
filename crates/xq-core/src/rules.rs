//! 终局判定与 perft 工具。

use crate::color::Color;
use crate::mv::{Move, MoveList};
use crate::piece::{PieceKind, kind_of};
use crate::position::Position;
use crate::square::to_iccs;

/// 判和的理由。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DrawReason {
    /// 60 回合自然限着（120 个半步内无吃子）。
    SixtyMoveRule,
    /// 双方均无进攻子力。
    InsufficientMaterial,
}

impl DrawReason {
    /// 面向用户的中文说明。
    pub const fn description(self) -> &'static str {
        match self {
            DrawReason::SixtyMoveRule => "60 回合内未吃子，自然限着判和",
            DrawReason::InsufficientMaterial => "双方均无进攻子力，无法将死对方，判和",
        }
    }
}

/// 局面状态。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameStatus {
    /// 对局进行中。
    Ongoing,
    /// 走子方正被将军，但仍有着法。
    Check { side: Color },
    /// 走子方被将死（无着可走且正被将军）。
    Checkmate { loser: Color },
    /// 困毙：走子方无着可走但**未**被将军。
    ///
    /// 中国象棋判**负**，与国际象棋判和相反 —— 这是两个规则体系最易混淆的差异点。
    Stalemate { loser: Color },
    /// 和棋。
    Draw { reason: DrawReason },
}

impl GameStatus {
    /// 对局是否已结束。
    pub const fn is_over(self) -> bool {
        matches!(
            self,
            GameStatus::Checkmate { .. } | GameStatus::Stalemate { .. } | GameStatus::Draw { .. }
        )
    }

    /// 面向用户的中文描述。
    pub const fn description(self) -> &'static str {
        match self {
            GameStatus::Ongoing => "对局进行中",
            GameStatus::Check { .. } => "将军",
            GameStatus::Checkmate { .. } => "将死",
            GameStatus::Stalemate { .. } => "困毙",
            GameStatus::Draw { .. } => "和棋",
        }
    }
}

/// 自然限着的半步阈值：60 回合 = 120 个半步。
pub const SIXTY_MOVE_HALF_MOVES: u16 = 120;

impl Position {
    /// 判定当前局面状态。
    ///
    /// 判定顺序（与设计文档 §5.1 一致）：
    /// ① 无着可走 → 将死 / 困毙（无论何种情况都优先）；
    /// ② 60 回合自然限着（**正值将军时延后**）；
    /// ③ 子力不足；
    /// ④ 将军；
    /// ⑤ 进行中。
    ///
    /// > **重复局面（长将 / 长捉）不在这里** —— 它需要回看整段对局历史，由
    /// > [`crate::repetition::adjudicate`] 单独给出裁决，对局层负责合并。
    /// > 这样 `status()` 保持为「只看当前局面」的纯函数。
    pub fn status(&mut self) -> GameStatus {
        let side = self.side_to_move();
        let in_check = self.is_in_check(side);

        if !self.has_legal_move() {
            // 中国象棋：无着可走即负，无论是否被将军。
            return if in_check {
                GameStatus::Checkmate { loser: side }
            } else {
                GameStatus::Stalemate { loser: side }
            };
        }

        // 例外：正值将军时限着判定延后，否则会出现「把对方将到最后一个回合
        // 就自动和棋」的荒谬结果。
        if self.halfmove_clock() >= SIXTY_MOVE_HALF_MOVES && !in_check {
            return GameStatus::Draw {
                reason: DrawReason::SixtyMoveRule,
            };
        }

        if self.is_insufficient_material() {
            return GameStatus::Draw {
                reason: DrawReason::InsufficientMaterial,
            };
        }

        if in_check {
            return GameStatus::Check { side };
        }

        GameStatus::Ongoing
    }

    /// 双方是否均无进攻子力（只有将帅 + 仕士 + 相象）。
    ///
    /// 采取**保守策略**：场上只要还存在兵 / 卒、马、车、炮中任意一个（无论哪方），
    /// 就不判和。因为即便是一个未过河的兵，理论上仍可助攻将死；误判和棋
    /// （把还能赢的判成和）比让对局继续严重得多。
    pub fn is_insufficient_material(&self) -> bool {
        for &p in self.squares().iter() {
            match kind_of(p) {
                None => {}
                Some(PieceKind::King | PieceKind::Advisor | PieceKind::Elephant) => {}
                Some(_) => return false,
            }
        }
        true
    }

    /// 该着法走完后是否将军了对方。
    ///
    /// > 需要走子后才能观察，因此本函数 make → 检测 → unmake。
    /// > **不要用在搜索热点路径** —— 搜索请直接读 `MoveRecord::gave_check`。
    pub fn move_gives_check(&mut self, mv: Move) -> bool {
        let us = self.side_to_move();
        self.make_move_unchecked(mv);
        let gives = self.is_attacked(self.king_index(us.opponent()), us);
        self.unmake_move_unchecked();
        gives
    }
}

/// 统计某深度下的合法着法路径总数（performance test）。
///
/// perft 是走法生成正确性的**最硬**验证手段：任何一处走法规则偏差都会让节点数
/// 与参考值不符，且误差随深度指数放大 —— 不存在「大部分对」的可能。
///
/// 参考值见 [docs/03](../../../docs/03-规则引擎与领域模型.md) §11.2：仅
/// `perft(1) = 44` 经手工逐子推导确认，更深的值须以本实现的实测值为准。
pub fn perft(pos: &mut Position, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let mut list = MoveList::new();
    pos.gen_legal_into(&mut list);
    if depth == 1 {
        return list.len() as u64;
    }
    let mut nodes = 0u64;
    for i in 0..list.len() {
        let mv = list.get(i);
        let record = pos.make_move_unchecked(mv);
        nodes += perft(pos, depth - 1);
        pos.unmake_move_unchecked();
        debug_assert_eq!(pos.hash(), record.prev_hash, "perft 回滚后哈希未还原");
    }
    nodes
}

/// 按根着法分组的 perft，用于定位是哪一分支的生成出了问题。
///
/// 返回 `(着法 ICCS 串, 该着法子树节点数)`，按节点数降序。
pub fn perft_divide(pos: &mut Position, depth: u32) -> Vec<(String, u64)> {
    let mut list = MoveList::new();
    pos.gen_legal_into(&mut list);
    let mut out = Vec::with_capacity(list.len());
    for i in 0..list.len() {
        let mv = list.get(i);
        let label = format!("{}{}", to_iccs(mv.from()), to_iccs(mv.to()));
        pos.make_move_unchecked(mv);
        let n = if depth <= 1 { 1 } else { perft(pos, depth - 1) };
        pos.unmake_move_unchecked();
        out.push((label, n));
    }
    out.sort_by_key(|(_, n)| core::cmp::Reverse(*n));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::piece::PieceKind;
    use crate::position::position_from_pieces;

    fn build(pieces: &[(Color, PieceKind, &str)], side: Color) -> Position {
        position_from_pieces(pieces, side).expect("测试局面构造失败")
    }

    /// perft(1) 必须等于设计文档手工推导的 44。
    #[test]
    fn perft_depth_one_is_44() {
        let mut pos = Position::startpos();
        assert_eq!(perft(&mut pos, 1), 44);
        pos.assert_consistent();
    }

    /// perft 跑完后局面必须逐字节还原。
    #[test]
    fn perft_restores_position() {
        let mut pos = Position::startpos();
        let before = pos.hash();
        let fen_before = crate::fen::to_fen(&pos);
        perft(&mut pos, 3);
        assert_eq!(pos.hash(), before);
        assert_eq!(crate::fen::to_fen(&pos), fen_before);
        assert_eq!(pos.ply(), 0);
        pos.assert_consistent();
    }

    #[test]
    fn startpos_status_is_ongoing() {
        let mut pos = Position::startpos();
        assert_eq!(pos.status(), GameStatus::Ongoing);
        assert!(!pos.is_in_check(Color::Red));
        assert!(!pos.is_in_check(Color::Black));
    }

    /// 将死：黑将 `e9` 被 `e8` 红车将军，`d9`/`f9` 分别被 `d8`/`f8` 车控制，
    /// 吃 `e8` 车则落点被 `d8`/`f8` 车攻击 —— 无处可逃。
    #[test]
    fn checkmate_is_detected() {
        let mut pos = build(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Chariot, "d8"),
                (Color::Red, PieceKind::Chariot, "e8"),
                (Color::Red, PieceKind::Chariot, "f8"),
                (Color::Black, PieceKind::King, "e9"),
            ],
            Color::Black,
        );
        assert!(pos.is_in_check(Color::Black), "黑方应被将军");
        assert!(!pos.has_legal_move(), "黑方应无着可走");
        assert_eq!(
            pos.status(),
            GameStatus::Checkmate {
                loser: Color::Black
            }
        );
    }

    /// 困毙：黑将 `d9` 无着可走但**未**被将军 —— 中国象棋判负。
    ///
    /// `d8` 被 `a8` 车沿第 8 行控制；`e9` 被 `e1` 车沿 e 列控制；
    /// `d9` 本身不被任何红子攻击。
    #[test]
    fn stalemate_loses_in_xiangqi() {
        let mut pos = build(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Chariot, "a8"),
                (Color::Red, PieceKind::Chariot, "e1"),
                (Color::Black, PieceKind::King, "d9"),
            ],
            Color::Black,
        );

        assert!(!pos.is_in_check(Color::Black), "困毙的前提是未被将军");
        assert!(!pos.has_legal_move(), "黑方应无着可走");
        assert_eq!(
            pos.status(),
            GameStatus::Stalemate {
                loser: Color::Black
            },
            "中国象棋中困毙判负，而非判和"
        );
    }

    /// 只有两个将帅 → 子力不足判和。
    #[test]
    fn bare_kings_is_draw() {
        let mut pos = build(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Black, PieceKind::King, "e9"),
            ],
            Color::Red,
        );
        assert!(pos.is_insufficient_material());
        assert_eq!(
            pos.status(),
            GameStatus::Draw {
                reason: DrawReason::InsufficientMaterial
            }
        );
    }

    /// 一个未过河的兵也不算子力不足 —— 保守策略。
    #[test]
    fn a_single_pawn_prevents_draw() {
        // 红帅放 d0：与黑将 e9 不同列，避免形成「白脸将」非法局面
        let mut pos = build(
            &[
                (Color::Red, PieceKind::King, "d0"),
                (Color::Red, PieceKind::Pawn, "a3"),
                (Color::Black, PieceKind::King, "e9"),
            ],
            Color::Red,
        );
        assert!(!pos.is_insufficient_material());
        assert!(!pos.is_in_check(Color::Red), "测试局面不应处于将军状态");
        assert_eq!(pos.status(), GameStatus::Ongoing);
    }

    /// 仕士相象均不构成进攻子力。
    #[test]
    fn advisors_and_elephants_are_not_material() {
        let pos = build(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Advisor, "d0"),
                (Color::Red, PieceKind::Elephant, "c0"),
                (Color::Black, PieceKind::King, "e9"),
                (Color::Black, PieceKind::Advisor, "d9"),
                (Color::Black, PieceKind::Elephant, "c9"),
            ],
            Color::Red,
        );
        assert!(pos.is_insufficient_material());
    }

    /// 60 回合自然限着判和（局面仍有大量子力，故不是子力不足和）。
    #[test]
    fn sixty_move_rule_draws() {
        let start = Position::startpos();
        let mut squares = *start.squares();
        // 移走两个红车与两个红炮，避免它们干扰（保留足量子力以排除子力不足）
        for coord in ["a0", "i0", "b2", "h2"] {
            squares[crate::square::from_iccs(coord).unwrap() as usize] = crate::piece::EMPTY;
        }
        let mut pos = Position::from_squares(squares, Color::Red, 120, 61).unwrap();
        assert!(!pos.is_insufficient_material());
        assert!(!pos.is_in_check(Color::Red));
        assert_eq!(
            pos.status(),
            GameStatus::Draw {
                reason: DrawReason::SixtyMoveRule
            }
        );
    }

    /// 将军状态下自然限着判定延后 —— 将死优先于限着判和。
    #[test]
    fn sixty_move_rule_is_deferred_while_in_check() {
        let base = build(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Chariot, "d8"),
                (Color::Red, PieceKind::Chariot, "e8"),
                (Color::Red, PieceKind::Chariot, "f8"),
                (Color::Black, PieceKind::King, "e9"),
            ],
            Color::Black,
        );
        let mut pos = Position::from_squares(*base.squares(), Color::Black, 120, 61).unwrap();
        assert!(pos.is_in_check(Color::Black));
        assert_eq!(
            pos.status(),
            GameStatus::Checkmate {
                loser: Color::Black
            },
            "将军状态下应优先判定将死，而非自然限着判和"
        );
    }
}
