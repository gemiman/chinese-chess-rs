//! 着法排序与历史启发。
//!
//! # 为什么排序如此重要
//!
//! Alpha-Beta 的效率**完全**取决于着法排序质量：最坏情况（逆序）节点数是指数级，
//! 最好情况（完美排序）是 `O(b^(d/2))`。实际引擎之间的强度差异，很大一部分来自排序。
//!
//! 优先级（[docs/04 §3.5](../../../docs/04-AI引擎设计.md)）：
//!
//! | 优先级 | 类别 |
//! |---|---|
//! | 1 | 置换表着法（上一轮迭代的最佳着法，命中率最高） |
//! | 2 | 吃子着法（MVV-LVA：用小子吃大子优先） |
//! | 3 | 杀手着法（同深度造成过 beta 截断的非吃子着法，保留 2 个） |
//! | 4 | 其余着法，按历史启发分排序 |
//!
//! # 选择排序而非全排序
//!
//! [`MovePicker`] 每次只挑出「当前剩余里分最高的一个」，而不是一次性排好序。
//! 这样当搜索在中途发生 beta 截断时，尚未挑选的着法根本不需要排序 ——
//! 省下的时间会直接变成搜索深度。

use xq_core::piece::{EMPTY, PieceKind, kind_of};
use xq_core::{MAX_MOVES, Move, MoveList, Position};

/// 着法排序用的子力价值。
///
/// 与 [`crate::eval`] 的基准值保持一致，但**不做阶段插值** —— 排序只关心相对大小，
/// 而每次插值都要先算阶段系数，代价不划算。
const fn order_value(kind: PieceKind) -> i32 {
    match kind {
        // 吃掉对方将帅在合法局面中不会出现，但排序表需要覆盖所有变体
        PieceKind::King => 10_000,
        PieceKind::Chariot => 900,
        PieceKind::Cannon => 450,
        PieceKind::Horse => 400,
        PieceKind::Advisor | PieceKind::Elephant => 200,
        PieceKind::Pawn => 100,
    }
}

// 各档基准分彼此拉开足够距离，保证「高档一定排在低档之前」
const SCORE_TT: i32 = 1_000_000;
const SCORE_CAPTURE_BASE: i32 = 200_000;
const SCORE_KILLER_1: i32 = 150_000;
const SCORE_KILLER_2: i32 = 140_000;

/// 历史启发表。
///
/// `history[from][to]` 累加某着法造成 beta 截断的次数，按 `depth²` 加权 ——
/// 深层造成的截断更有价值。超过阈值时整体右移，抑制溢出同时保留相对大小。
#[derive(Clone)]
pub struct History {
    table: Vec<i32>,
    max: i32,
}

impl History {
    /// 新建空表。
    pub fn new() -> Self {
        Self {
            table: vec![0; 90 * 90],
            max: 0,
        }
    }

    /// 清空（换局时调用）。
    pub fn clear(&mut self) {
        self.table.iter_mut().for_each(|v| *v = 0);
        self.max = 0;
    }

    /// 取分。
    #[inline]
    pub fn get(&self, from: u8, to: u8) -> i32 {
        self.table[from as usize * 90 + to as usize]
    }

    /// 加分。
    pub fn bump(&mut self, from: u8, to: u8, depth: i32) {
        let idx = from as usize * 90 + to as usize;
        let gain = (depth * depth).clamp(1, 4096);
        self.table[idx] += gain;
        if self.table[idx] > self.max {
            self.max = self.table[idx];
        }
        if self.max > 1 << 20 {
            for v in self.table.iter_mut() {
                *v >>= 1;
            }
            self.max >>= 1;
        }
    }

    /// 按世代衰减，避免旧局面的数据长期污染排序。
    pub fn decay(&mut self) {
        for v in self.table.iter_mut() {
            *v >>= 3;
        }
        self.max >>= 3;
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for History {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("History").field("max", &self.max).finish()
    }
}

/// 增量式着法选择器。
pub struct MovePicker {
    /// 自有缓冲区：`MoveList` 的内部数组不对外开放，而选择排序需要就地交换。
    moves: [Move; MAX_MOVES],
    scores: [i32; MAX_MOVES],
    len: usize,
    cursor: usize,
}

impl MovePicker {
    /// 生成并打分当前局面的全部合法着法。
    pub fn new(
        pos: &mut Position,
        tt_move: Option<Move>,
        killers: &[Move; 2],
        history: &History,
    ) -> Self {
        let mut generated = MoveList::new();
        pos.gen_legal_into(&mut generated);

        let mut moves = [Move(0); MAX_MOVES];
        let mut scores = [0i32; MAX_MOVES];
        let len = generated.len();
        for i in 0..len {
            let mv = generated.get(i);
            moves[i] = mv;
            scores[i] = score_move(pos, mv, tt_move, killers, history);
        }

        Self {
            moves,
            scores,
            len,
            cursor: 0,
        }
    }

    /// 着法总数。
    pub fn len(&self) -> usize {
        self.len
    }

    /// 是否没有着法（说明该局面是将死或困毙）。
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 取出下一个着法（只挑当前剩余里分最高的那个）。
    pub fn pick(&mut self) -> Option<Move> {
        if self.cursor >= self.len {
            return None;
        }
        let mut best = self.cursor;
        for i in (self.cursor + 1)..self.len {
            if self.scores[i] > self.scores[best] {
                best = i;
            }
        }
        self.moves.swap(self.cursor, best);
        self.scores.swap(self.cursor, best);
        let mv = self.moves[self.cursor];
        self.cursor += 1;
        Some(mv)
    }

    /// 剩余着法数量。
    pub fn remaining(&self) -> usize {
        self.len.saturating_sub(self.cursor)
    }
}

impl core::fmt::Debug for MovePicker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MovePicker")
            .field("len", &self.len)
            .field("cursor", &self.cursor)
            .finish()
    }
}

/// 给单个着法打分。
fn score_move(
    pos: &Position,
    mv: Move,
    tt_move: Option<Move>,
    killers: &[Move; 2],
    history: &History,
) -> i32 {
    if Some(mv) == tt_move {
        return SCORE_TT;
    }

    let target = pos.piece_at(mv.to());
    if target != EMPTY {
        // MVV-LVA：受害子价值 − 攻击子价值。用小子吃大子排前面。
        let victim = kind_of(target).map(order_value).unwrap_or(0);
        let attacker = kind_of(pos.piece_at(mv.from()))
            .map(order_value)
            .unwrap_or(0);
        // ×16 保证「受害子差一档」的影响远大于「攻击子价值」的细微差别
        return SCORE_CAPTURE_BASE + victim * 16 - attacker;
    }

    if mv == killers[0] {
        return SCORE_KILLER_1;
    }
    if mv == killers[1] {
        return SCORE_KILLER_2;
    }

    // 其余着法按历史分排序。历史分不会超过 ~1<<20，但为保险起见夹到杀手档之下。
    history.get(mv.from(), mv.to()).clamp(0, SCORE_KILLER_2 - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xq_core::Color;
    use xq_core::square::from_iccs;

    fn mv(iccs: &str) -> Move {
        Move::new(
            from_iccs(&iccs[0..2]).unwrap(),
            from_iccs(&iccs[2..4]).unwrap(),
        )
    }

    /// 一个没有任何吃子机会的局面，用于干净地测「非吃子着法的排序」。
    fn quiet_position() -> Position {
        xq_core::position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Horse, "b0"),
                (Color::Red, PieceKind::Chariot, "a0"),
                // 黑将放 d9：与红帅 e0 不同列，避免形成「白脸将」非法局面
                (Color::Black, PieceKind::King, "d9"),
            ],
            Color::Red,
        )
        .unwrap()
    }

    #[test]
    fn history_bump_and_get() {
        let mut h = History::new();
        assert_eq!(h.get(0, 1), 0);
        h.bump(0, 1, 4);
        assert_eq!(h.get(0, 1), 16); // depth²
        h.bump(0, 1, 2);
        assert_eq!(h.get(0, 1), 20);
    }

    #[test]
    fn history_decay_shrinks() {
        let mut h = History::new();
        h.bump(0, 1, 8);
        let before = h.get(0, 1);
        h.decay();
        assert!(h.get(0, 1) < before);
    }

    /// 置换表着法必须排在最前。
    #[test]
    fn tt_move_comes_first() {
        let mut pos = Position::startpos();
        let tt = mv("h2e2");
        let killers = [Move(0), Move(0)];
        let history = History::new();

        let mut picker = MovePicker::new(&mut pos, Some(tt), &killers, &history);
        assert_eq!(picker.len(), 44);
        assert_eq!(picker.pick(), Some(tt));
    }

    /// MVV-LVA：吃子着法排在非吃子之前。
    #[test]
    fn captures_come_before_quiet_moves() {
        // 红兵 b3 可以向前吃掉 b4 的黑车；红车 a0 只能走到空位
        let mut pos = xq_core::position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Pawn, "b3"),
                (Color::Red, PieceKind::Chariot, "a0"),
                (Color::Black, PieceKind::King, "d9"),
                (Color::Black, PieceKind::Chariot, "b4"),
            ],
            Color::Red,
        )
        .unwrap();

        let pawn_takes_chariot = mv("b3b4");
        let quiet_rook_move = mv("a0a1");

        let killers = [Move(0), Move(0)];
        let history = History::new();
        let mut picker = MovePicker::new(&mut pos, None, &killers, &history);
        let first = picker.pick().unwrap();
        assert_eq!(first, pawn_takes_chariot, "兵吃车应排第一");
        assert_ne!(first, quiet_rook_move);
    }

    /// 杀手着法应排在所有普通着法之前。
    #[test]
    fn killers_before_quiet_moves() {
        let mut pos = quiet_position();
        let killer = mv("b0c2");
        let killers = [killer, Move(0)];
        let history = History::new();

        let mut picker = MovePicker::new(&mut pos, None, &killers, &history);
        assert!(picker.len() > 1);
        assert_eq!(picker.pick().unwrap(), killer, "无吃子时，杀手着法应排第一");
    }

    /// 历史分高的着法应排在历史分低的之前。
    #[test]
    fn history_orders_quiet_moves() {
        let mut pos = quiet_position();
        let killers = [Move(0), Move(0)];
        let mut history = History::new();
        let favoured = mv("b0a2");
        history.bump(favoured.from(), favoured.to(), 6);

        let mut picker = MovePicker::new(&mut pos, None, &killers, &history);
        assert_eq!(picker.pick().unwrap(), favoured);
    }

    /// 选择器必须不重不漏地给出全部着法。
    #[test]
    fn picker_yields_all_moves_exactly_once() {
        let mut pos = Position::startpos();
        let killers = [Move(0), Move(0)];
        let history = History::new();
        let mut picker = MovePicker::new(&mut pos, None, &killers, &history);

        let total = picker.len();
        let mut seen = std::collections::HashSet::new();
        while let Some(m) = picker.pick() {
            assert!(seen.insert(m), "着法 {m} 被重复产出");
        }
        assert_eq!(seen.len(), total);
        assert_eq!(total, 44);
        assert_eq!(picker.remaining(), 0);
        assert_eq!(picker.pick(), None);
    }
}
