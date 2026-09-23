//! 伪合法着法生成。
//!
//! # 两阶段架构
//!
//! ```text
//! 第一阶段（本模块）：生成伪合法着法
//!     —— 只考虑棋子自身走法（蹩马腿 / 塞象眼 / 炮架 / 九宫 / 河界）
//!     —— 不检查「走完后己方将帅是否被将」
//! 第二阶段（rules / Position::gen_legal_into）：make → 检测 → unmake → 过滤
//! ```
//!
//! **为什么不做「预生成对方攻击位图」**：中国象棋中炮与马的攻击关系依赖实时
//! 局面（炮依赖炮架、马依赖蹩腿点），无法静态穷举。`make/unmake + 攻击检测`
//! 实现简单、正确性易验证，且被主流引擎广泛采用。

use crate::color::Color;
use crate::mv::{Move, MoveList};
use crate::piece::{PieceKind, color_of, kind_of};
use crate::position::Position;
use crate::square::{
    BOARD_SIZE, DIAG, ORTHO, col_of, has_crossed_river, in_palace, index, on_board, on_own_side,
    row_of,
};

/// 马的走法表：`(Δ列, Δ行, 蹩腿点Δ列, 蹩腿点Δ行)`，均相对马自身。
const HORSE_STEPS: [(i8, i8, i8, i8); 8] = [
    (1, 2, 0, 1),
    (-1, 2, 0, 1),
    (1, -2, 0, -1),
    (-1, -2, 0, -1),
    (2, 1, 1, 0),
    (2, -1, 1, 0),
    (-2, 1, -1, 0),
    (-2, -1, -1, 0),
];

/// 相 / 象的走法：四个田字斜向。象眼位于 `(Δ列/2, Δ行/2)`。
const ELEPHANT_STEPS: [(i8, i8); 4] = [(2, 2), (2, -2), (-2, 2), (-2, -2)];

impl Position {
    /// 生成当前走子方的全部伪合法着法。
    pub fn gen_pseudo_into(&self, out: &mut MoveList) {
        out.clear();
        let us = self.side_to_move();
        let squares = self.squares();

        for idx in 0..BOARD_SIZE as u8 {
            let piece = squares[idx as usize];
            if color_of(piece) != Some(us) {
                continue;
            }
            let kind = kind_of(piece).expect("非空棋子必有种类");
            let col = col_of(idx) as i8;
            let row = row_of(idx) as i8;

            match kind {
                PieceKind::Chariot => self.gen_chariot(out, idx, col, row, us),
                PieceKind::Cannon => self.gen_cannon(out, idx, col, row, us),
                PieceKind::Horse => self.gen_horse(out, idx, col, row, us),
                PieceKind::Elephant => self.gen_elephant(out, idx, col, row, us),
                PieceKind::Advisor => self.gen_advisor(out, idx, col, row, us),
                PieceKind::King => self.gen_king(out, idx, col, row, us),
                PieceKind::Pawn => self.gen_pawn(out, idx, col, row, us),
            }
        }
    }

    /// 该格是否可落子：不能是己方棋子，也**不能是对方将帅**。
    ///
    /// 排除「吃将帅」有两个原因：
    ///
    /// 1. **规则上不存在这一步** —— 将死即终局，棋局不以吃掉将帅收尾；
    /// 2. **防悬空指针** —— 从合法对局出发这情形永不出现，但手工构造的 FEN
    ///    可以摆出「对方将帅正被吃」的局面。若不排除，`legal_moves()` 会返回
    ///    一步吃掉对方将帅的着法，走完之后将帅缓存就指向了一个已被占领的格子，
    ///    触发一致性断言失败。
    #[inline]
    fn can_land_on(&self, idx: u8, us: Color) -> bool {
        let p = self.squares()[idx as usize];
        if p == crate::piece::EMPTY {
            return true;
        }
        if color_of(p) == Some(us) {
            return false;
        }
        kind_of(p) != Some(PieceKind::King)
    }

    /// 车：四方向射线，遇第一个棋子时敌子可吃、己子不可走，然后终止。
    fn gen_chariot(&self, out: &mut MoveList, from: u8, col: i8, row: i8, us: Color) {
        for &(dc, dr) in ORTHO.iter() {
            let mut c = col + dc;
            let mut r = row + dr;
            while on_board(c, r) {
                let to = index(c as u8, r as u8);
                let target = self.squares()[to as usize];
                if target == crate::piece::EMPTY {
                    out.push(Move::new(from, to));
                } else {
                    if self.can_land_on(to, us) {
                        out.push(Move::new(from, to));
                    }
                    break;
                }
                c += dc;
                r += dr;
            }
        }
    }

    /// 炮：移动与吃子规则不同。
    ///
    /// - **移动**（目标为空）：照车的方式，遇第一个棋子前的所有空格均可走；
    /// - **吃子**（目标为敌子）：起点与目标之间必须**恰好有一个棋子**（炮架，
    ///   任意方均可）。
    fn gen_cannon(&self, out: &mut MoveList, from: u8, col: i8, row: i8, us: Color) {
        for &(dc, dr) in ORTHO.iter() {
            // 阶段一：空格可走，遇到第一个棋子停下（该格作为炮架，不落子）
            let mut c = col + dc;
            let mut r = row + dr;
            while on_board(c, r) {
                let to = index(c as u8, r as u8);
                if self.squares()[to as usize] != crate::piece::EMPTY {
                    break;
                }
                out.push(Move::new(from, to));
                c += dc;
                r += dr;
            }

            // 阶段二：越过炮架后，寻第一个棋子
            if !on_board(c, r) {
                continue;
            }
            c += dc;
            r += dr;
            while on_board(c, r) {
                let to = index(c as u8, r as u8);
                let target = self.squares()[to as usize];
                if target != crate::piece::EMPTY {
                    if self.can_land_on(to, us) {
                        out.push(Move::new(from, to));
                    }
                    break;
                }
                c += dc;
                r += dr;
            }
        }
    }

    /// 马：八个目标，各有一个蹩腿点；蹩腿点必须是**空格**，否则该方向不可走。
    ///
    /// > 注意：蹩腿点只需为空，与目标点是什么棋子无关。
    fn gen_horse(&self, out: &mut MoveList, from: u8, col: i8, row: i8, us: Color) {
        for &(dc, dr, lc, lr) in HORSE_STEPS.iter() {
            let c = col + dc;
            let r = row + dr;
            if !on_board(c, r) {
                continue;
            }
            let leg_c = col + lc;
            let leg_r = row + lr;
            debug_assert!(on_board(leg_c, leg_r), "蹩腿点必然在棋盘内");
            if self.squares()[index(leg_c as u8, leg_r as u8) as usize] != crate::piece::EMPTY {
                continue;
            }
            let to = index(c as u8, r as u8);
            if self.can_land_on(to, us) {
                out.push(Move::new(from, to));
            }
        }
    }

    /// 相 / 象：四个田字斜向；**象眼**（田字中心）必须是空格；**不可过河**。
    fn gen_elephant(&self, out: &mut MoveList, from: u8, col: i8, row: i8, us: Color) {
        for &(dc, dr) in ELEPHANT_STEPS.iter() {
            let c = col + dc;
            let r = row + dr;
            if !on_board(c, r) {
                continue;
            }
            // 不可过河
            if !on_own_side(us, r as u8) {
                continue;
            }
            // 塞象眼
            let eye = index((col + dc / 2) as u8, (row + dr / 2) as u8);
            if self.squares()[eye as usize] != crate::piece::EMPTY {
                continue;
            }
            let to = index(c as u8, r as u8);
            if self.can_land_on(to, us) {
                out.push(Move::new(from, to));
            }
        }
    }

    /// 仕 / 士：四个斜向一步，目标须在己方九宫内。
    fn gen_advisor(&self, out: &mut MoveList, from: u8, col: i8, row: i8, us: Color) {
        for &(dc, dr) in DIAG.iter() {
            let c = col + dc;
            let r = row + dr;
            if !on_board(c, r) || !in_palace(us, c as u8, r as u8) {
                continue;
            }
            let to = index(c as u8, r as u8);
            if self.can_land_on(to, us) {
                out.push(Move::new(from, to));
            }
        }
    }

    /// 帅 / 将：四个直向一步，目标须在己方九宫内。
    ///
    /// 白脸将不在这里处理 —— 它由统一攻击检测覆盖（见
    /// [`Position::is_attacked`]），走完后若将帅照面会被合法性过滤剔除。
    fn gen_king(&self, out: &mut MoveList, from: u8, col: i8, row: i8, us: Color) {
        for &(dc, dr) in ORTHO.iter() {
            let c = col + dc;
            let r = row + dr;
            if !on_board(c, r) || !in_palace(us, c as u8, r as u8) {
                continue;
            }
            let to = index(c as u8, r as u8);
            if self.can_land_on(to, us) {
                out.push(Move::new(from, to));
            }
        }
    }

    /// 兵 / 卒：永远向前一步；**过河后**额外获得左右横走一步的能力。
    ///
    /// 兵卒永不后退，走到对方底线后横向能力保留（俗称「老兵」）。
    fn gen_pawn(&self, out: &mut MoveList, from: u8, col: i8, row: i8, us: Color) {
        // 向前
        let fr = row + us.forward();
        if on_board(col, fr) {
            let to = index(col as u8, fr as u8);
            if self.can_land_on(to, us) {
                out.push(Move::new(from, to));
            }
        }
        // 过河后可横向
        if has_crossed_river(us, row as u8) {
            for dc in [-1i8, 1] {
                let c = col + dc;
                if !on_board(c, row) {
                    continue;
                }
                let to = index(c as u8, row as u8);
                if self.can_land_on(to, us) {
                    out.push(Move::new(from, to));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::square::to_iccs;

    fn pseudo_from_startpos() -> (Position, MoveList) {
        let pos = Position::startpos();
        let mut list = MoveList::new();
        pos.gen_pseudo_into(&mut list);
        (pos, list)
    }

    /// 初始局面伪合法着法数 = 44（初始局面不存在自将，故伪合法 == 合法）。
    #[test]
    fn startpos_pseudo_legal_count_is_44() {
        let (_, list) = pseudo_from_startpos();
        assert_eq!(
            list.len(),
            44,
            "初始局面伪合法着法数应为 44，实际 {}",
            list.len()
        );
    }

    /// 逐棋子回归设计文档 §11.2 的手工推导表。
    #[test]
    fn startpos_per_piece_breakdown_matches_doc() {
        let (pos, list) = pseudo_from_startpos();

        let count_by_kind = |kind: PieceKind| -> usize {
            list.iter()
                .filter(|mv| kind_of(pos.piece_at(mv.from())) == Some(kind))
                .count()
        };

        assert_eq!(count_by_kind(PieceKind::Chariot), 4, "车 × 2 应为 4 步");
        assert_eq!(count_by_kind(PieceKind::Horse), 4, "马 × 2 应为 4 步");
        assert_eq!(count_by_kind(PieceKind::Elephant), 4, "相 × 2 应为 4 步");
        assert_eq!(count_by_kind(PieceKind::Advisor), 2, "仕 × 2 应为 2 步");
        assert_eq!(count_by_kind(PieceKind::King), 1, "帅应为 1 步");
        assert_eq!(
            count_by_kind(PieceKind::Cannon),
            24,
            "炮 × 2 应为 24 步（含吃 b9 黑马与 h9 黑马两个隔子吃；注意黑方底线是 rnbakabnr，h9 是马不是车）"
        );
        assert_eq!(count_by_kind(PieceKind::Pawn), 5, "兵 × 5 应为 5 步");
    }

    /// b2 炮的完整着法清单（文档 §11.2 中 12 步的逐项验证）。
    #[test]
    fn cannon_on_b2_has_twelve_moves() {
        let (pos, list) = pseudo_from_startpos();
        let from = crate::square::from_iccs("b2").unwrap();
        let mut targets: Vec<String> = list
            .iter()
            .filter(|mv| mv.from() == from)
            .map(|mv| to_iccs(mv.to()))
            .collect();
        targets.sort();

        assert_eq!(targets.len(), 12, "b2 炮应有 12 步，实际 {targets:?}");
        // 上移 4 步
        for t in ["b3", "b4", "b5", "b6"] {
            assert!(targets.contains(&t.to_string()), "缺上移 {t}");
        }
        // 隔子吃 b9 黑马（炮架 b7 黑炮）
        assert!(targets.contains(&"b9".to_string()), "缺隔子吃 b9 黑马");
        // 下移 1 步、左移 1 步、右移 5 步
        assert!(targets.contains(&"b1".to_string()), "缺下移 b1");
        assert!(targets.contains(&"a2".to_string()), "缺左移 a2");
        for t in ["c2", "d2", "e2", "f2", "g2"] {
            assert!(targets.contains(&t.to_string()), "缺右移 {t}");
        }
        // 不可吃 b7（同列相邻敌子，但无炮架）—— 这是最容易写错的一项
        assert!(
            !targets.contains(&"b7".to_string()),
            "b2 炮不应能吃到 b7（无炮架）"
        );
        let _ = pos;
    }

    /// 蹩马腿：初始局面 b0 马的 `d1` 被 c0 相蹩住。
    #[test]
    fn horse_is_blocked_by_leg() {
        let (_, list) = pseudo_from_startpos();
        let from = crate::square::from_iccs("b0").unwrap();
        let targets: Vec<String> = list
            .iter()
            .filter(|mv| mv.from() == from)
            .map(|mv| to_iccs(mv.to()))
            .collect();

        assert!(targets.contains(&"a2".to_string()), "b0 马应能到 a2");
        assert!(targets.contains(&"c2".to_string()), "b0 马应能到 c2");
        assert!(
            !targets.contains(&"d1".to_string()),
            "b0 马不应能到 d1（被 c0 相蹩腿）"
        );
    }

    /// 塞象眼：红相被己方棋子塞住眼时不能走。
    #[test]
    fn elephant_is_blocked_by_eye() {
        // 红相在 c0，象眼 b1 放一个红兵
        let pos = crate::position::position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Elephant, "c0"),
                (Color::Red, PieceKind::Pawn, "b1"),
                (Color::Black, PieceKind::King, "e9"),
            ],
            Color::Red,
        )
        .unwrap();
        let mut list = MoveList::new();
        pos.gen_pseudo_into(&mut list);
        let from = crate::square::from_iccs("c0").unwrap();
        let targets: Vec<String> = list
            .iter()
            .filter(|mv| mv.from() == from)
            .map(|mv| to_iccs(mv.to()))
            .collect();

        assert!(
            !targets.contains(&"a2".to_string()),
            "象眼被塞，不应能到 a2"
        );
        assert!(
            targets.contains(&"e2".to_string()),
            "另一侧象眼通畅，应能到 e2"
        );
    }

    /// 相不可过河：红相只能到达 7 个固定点。
    #[test]
    fn elephant_cannot_cross_river() {
        let pos = crate::position::position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "e0"),
                // 放在河界附近，四方向之一指向河对岸
                (Color::Red, PieceKind::Elephant, "e4"),
                (Color::Black, PieceKind::King, "e9"),
            ],
            Color::Red,
        )
        .unwrap();
        let mut list = MoveList::new();
        pos.gen_pseudo_into(&mut list);
        let from = crate::square::from_iccs("e4").unwrap();
        let targets: Vec<String> = list
            .iter()
            .filter(|mv| mv.from() == from)
            .map(|mv| to_iccs(mv.to()))
            .collect();

        // e4 的四目标：c2/g2/e2/e6，其中 e6 在河对岸（row 6），应被剔除
        assert!(targets.contains(&"c2".to_string()));
        assert!(targets.contains(&"g2".to_string()));
        assert!(
            !targets.contains(&"e6".to_string()),
            "相不可过河，不应能到 e6"
        );
    }

    /// 兵未过河不能横走，过河后可横走且永不后退。
    #[test]
    fn pawn_movement_depends_on_river() {
        let pos = crate::position::position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Pawn, "c3"), // 未过河（红方 row 3 <= 4）
                (Color::Red, PieceKind::Pawn, "c6"), // 已过河
                (Color::Black, PieceKind::King, "e9"),
            ],
            Color::Red,
        )
        .unwrap();
        let mut list = MoveList::new();
        pos.gen_pseudo_into(&mut list);

        let targets_of = |iccs: &str| -> Vec<String> {
            let from = crate::square::from_iccs(iccs).unwrap();
            list.iter()
                .filter(|mv| mv.from() == from)
                .map(|mv| to_iccs(mv.to()))
                .collect()
        };

        let before = targets_of("c3");
        assert_eq!(before, vec!["c4".to_string()], "未过河的兵只能前进一步");

        let after = targets_of("c6");
        assert!(after.contains(&"c7".to_string()), "过河兵应能前进");
        assert!(after.contains(&"b6".to_string()), "过河兵应能左移");
        assert!(after.contains(&"d6".to_string()), "过河兵应能右移");
        assert!(!after.contains(&"c5".to_string()), "兵永不后退");
    }

    /// 仕 / 帅被限制在九宫内。
    #[test]
    fn advisor_and_king_are_confined_to_palace() {
        let targets_of = |pieces: &[(Color, PieceKind, &str)], from: &str| -> Vec<String> {
            let pos = crate::position::position_from_pieces(pieces, Color::Red).unwrap();
            let mut list = MoveList::new();
            pos.gen_pseudo_into(&mut list);
            let idx = crate::square::from_iccs(from).unwrap();
            list.iter()
                .filter(|mv| mv.from() == idx)
                .map(|mv| to_iccs(mv.to()))
                .collect()
        };

        // 仕在 d0：九宫内的斜向落点只有 e1（c1 的 col = 2，在九宫外）
        let advisor = targets_of(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Advisor, "d0"),
                // 黑将放 d9：与红帅 e0 不同列，避免形成照面的非法局面
                (Color::Black, PieceKind::King, "d9"),
            ],
            "d0",
        );
        assert_eq!(advisor, vec!["e1".to_string()], "d0 仕在九宫内只剩 e1 可走");

        // 帅在 d0：可走 e0 与 d1；c0 的 col = 2 在九宫外
        let king = targets_of(
            &[
                (Color::Red, PieceKind::King, "d0"),
                (Color::Black, PieceKind::King, "e9"),
            ],
            "d0",
        );
        assert!(
            !king.contains(&"c0".to_string()),
            "帅不可走出九宫（c0 在九宫外）"
        );
        let mut sorted = king.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["d1".to_string(), "e0".to_string()]);
    }

    /// 车射线被己方阻挡、可吃敌方第一个子。
    #[test]
    fn chariot_ray_stops_and_captures() {
        let pos = crate::position::position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Chariot, "a0"),
                (Color::Red, PieceKind::Pawn, "a5"),   // 己方阻挡
                (Color::Black, PieceKind::Pawn, "d0"), // 敌方，可吃
                (Color::Black, PieceKind::King, "e9"),
            ],
            Color::Red,
        )
        .unwrap();
        let mut list = MoveList::new();
        pos.gen_pseudo_into(&mut list);
        let from = crate::square::from_iccs("a0").unwrap();
        let targets: Vec<String> = list
            .iter()
            .filter(|mv| mv.from() == from)
            .map(|mv| to_iccs(mv.to()))
            .collect();

        for t in ["a1", "a2", "a3", "a4"] {
            assert!(targets.contains(&t.to_string()), "应能到 {t}");
        }
        assert!(
            !targets.contains(&"a5".to_string()),
            "己方棋子不可吃也不可越过"
        );
        assert!(targets.contains(&"b0".to_string()));
        assert!(targets.contains(&"c0".to_string()));
        assert!(targets.contains(&"d0".to_string()), "应能吃敌子 d0");
        assert_eq!(targets.len(), 7);
    }
}
