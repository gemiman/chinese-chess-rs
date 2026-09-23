//! 局面表示、`make_move` / `unmake_move` 与攻击检测。
//!
//! # 表示法
//!
//! 一维 `[u8; 90]` 数组是**唯一真相源**，两个 `u128` 占据位图是**派生数据**，
//! 只在 `do_move` / `undo` 中增量维护。位图的作用是让「该格是否为空」退化为
//! 一次位测试。
//!
//! > 不采用纯位棋盘：中国象棋的「炮」是「隔一子吃」语义，与位棋盘的
//! > 「集合交并」模型不适配，强行实现会引入大量临时位运算且极难调试。

use crate::color::Color;
use crate::mv::{MAX_MOVES, Move, MoveList, MoveRecord};
use crate::piece::{EMPTY, PieceKind, color_of, encode, kind_of, zobrist_index};
use crate::square::{
    BOARD_SIZE, COLS, ORTHO, ROWS, col_of, in_palace, index, is_valid_index, on_board, row_of,
};
use crate::zobrist::ZOBRIST;

/// 局面构造失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PositionError {
    /// 缺少某方的将帅。
    MissingKing(Color),
    /// 同一方出现多个将帅。
    DuplicateKing(Color),
}

impl core::fmt::Display for PositionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PositionError::MissingKing(c) => write!(f, "局面缺少{}方将帅", c.name_zh()),
            PositionError::DuplicateKing(c) => write!(f, "局面出现多个{}方将帅", c.name_zh()),
        }
    }
}

impl std::error::Error for PositionError {}

/// 非法着法。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IllegalMove {
    /// `from == to`。
    Null,
    /// 起点或终点越界。
    OffBoard { from: u8, to: u8 },
    /// 起点没有棋子。
    NoPiece { from: u8 },
    /// 起点的棋子不属于当前走子方。
    WrongSide { from: u8, expected: Color },
    /// 走完后己方将帅被攻击（含白脸将）。
    LeavesKingInCheck,
    /// 不满足棋子自身的走法规则（路径阻挡、蹩马腿、塞象眼、九宫/河界限制等）。
    NotLegal { from: u8, to: u8 },
}

impl core::fmt::Display for IllegalMove {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            IllegalMove::Null => write!(f, "起点与终点相同"),
            IllegalMove::OffBoard { from, to } => write!(
                f,
                "着法越界: {} → {}",
                crate::square::to_iccs(*from),
                crate::square::to_iccs(*to)
            ),
            IllegalMove::NoPiece { from } => {
                write!(f, "起点 {} 没有棋子", crate::square::to_iccs(*from))
            }
            IllegalMove::WrongSide { from, expected } => write!(
                f,
                "起点 {} 的棋子不属于{}方",
                crate::square::to_iccs(*from),
                expected.name_zh()
            ),
            IllegalMove::LeavesKingInCheck => write!(f, "该着法会导致己方将帅被将（含白脸将）"),
            IllegalMove::NotLegal { from, to } => write!(
                f,
                "{} → {} 不满足棋子走法规则（路径阻挡 / 蹩马腿 / 塞象眼 / 九宫河界限制）",
                crate::square::to_iccs(*from),
                crate::square::to_iccs(*to)
            ),
        }
    }
}

impl std::error::Error for IllegalMove {}

/// 一个完整局面。
///
/// 裸结构（不含 `history` / `move_stack` 两个 `Vec`）约 140 字节。
#[derive(Clone)]
pub struct Position {
    /// 90 格棋盘，唯一真相源。
    squares: [u8; BOARD_SIZE],
    /// 红方占据位图（派生）。
    red_occ: u128,
    /// 黑方占据位图（派生）。
    black_occ: u128,
    /// 当前走子方。
    side_to_move: Color,
    /// 红方将帅所在格（缓存）。
    red_king: u8,
    /// 黑方将帅所在格（缓存）。
    black_king: u8,
    /// 距上次吃子的半步数（60 回合自然限着用）。
    halfmove_clock: u16,
    /// 回合数。
    fullmove_number: u16,
    /// Zobrist 哈希（增量维护）。
    hash: u64,
    /// 已出现过的局面哈希序列，`history[0]` 是初始局面。
    history: Vec<u64>,
    /// 着法历史。
    move_stack: Vec<MoveRecord>,
}

impl Position {
    /// 由棋盘数组与元信息构造，自动推导位图、将帅缓存与哈希。
    ///
    /// 调用方须自行保证棋盘合法；本函数只校验将帅的存在性与唯一性。
    pub fn from_squares(
        squares: [u8; BOARD_SIZE],
        side_to_move: Color,
        halfmove_clock: u16,
        fullmove_number: u16,
    ) -> Result<Position, PositionError> {
        let mut red_occ = 0u128;
        let mut black_occ = 0u128;
        let mut red_king = None;
        let mut black_king = None;

        for (i, &p) in squares.iter().enumerate() {
            match color_of(p) {
                None => continue,
                Some(c) => {
                    let occ = match c {
                        Color::Red => &mut red_occ,
                        Color::Black => &mut black_occ,
                    };
                    *occ |= 1u128 << i;
                }
            }
            if kind_of(p) == Some(PieceKind::King) {
                let slot = match color_of(p).expect("非空棋子必有颜色") {
                    Color::Red => &mut red_king,
                    Color::Black => &mut black_king,
                };
                if slot.is_some() {
                    return Err(PositionError::DuplicateKing(
                        color_of(p).expect("非空棋子必有颜色"),
                    ));
                }
                *slot = Some(i as u8);
            }
        }

        let red_king = red_king.ok_or(PositionError::MissingKing(Color::Red))?;
        let black_king = black_king.ok_or(PositionError::MissingKing(Color::Black))?;
        let hash = Self::compute_hash(&squares, side_to_move);

        Ok(Position {
            squares,
            red_occ,
            black_occ,
            side_to_move,
            red_king,
            black_king,
            halfmove_clock,
            fullmove_number,
            hash,
            history: {
                let mut v = Vec::with_capacity(256);
                v.push(hash);
                v
            },
            move_stack: Vec::with_capacity(256),
        })
    }

    /// 标准初始局面。
    pub fn startpos() -> Position {
        crate::fen::from_fen(crate::fen::STARTPOS_FEN).expect("内置初始 FEN 必须可解析")
    }

    // ---------------------------------------------------------------- 查询

    /// 当前走子方。
    #[inline]
    pub const fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    /// 取某格上的棋子编码（0 = 空）。
    #[inline]
    pub fn piece_at(&self, idx: u8) -> u8 {
        debug_assert!(is_valid_index(idx));
        self.squares[idx as usize]
    }

    /// 某格是否有棋子。
    #[inline]
    pub fn is_occupied(&self, idx: u8) -> bool {
        debug_assert!(is_valid_index(idx));
        let bit = 1u128 << idx;
        (self.red_occ | self.black_occ) & bit != 0
    }

    /// 当前局面的 Zobrist 哈希。
    #[inline]
    pub const fn hash(&self) -> u64 {
        self.hash
    }

    /// 半步计数。
    #[inline]
    pub const fn halfmove_clock(&self) -> u16 {
        self.halfmove_clock
    }

    /// 回合数。
    #[inline]
    pub const fn fullmove_number(&self) -> u16 {
        self.fullmove_number
    }

    /// 只读访问整块棋盘。
    #[inline]
    pub const fn squares(&self) -> &[u8; BOARD_SIZE] {
        &self.squares
    }

    /// 已出现过的局面哈希序列（含初始局面）。
    #[inline]
    pub fn history(&self) -> &[u64] {
        &self.history
    }

    /// 着法历史。
    #[inline]
    pub fn move_stack(&self) -> &[MoveRecord] {
        &self.move_stack
    }

    /// 已走的步数。
    #[inline]
    pub fn ply(&self) -> usize {
        self.move_stack.len()
    }

    /// 某方将帅所在格。
    #[inline]
    pub fn king_index(&self, color: Color) -> u8 {
        match color {
            Color::Red => self.red_king,
            Color::Black => self.black_king,
        }
    }

    /// 某方将帅是否位于己方九宫内（合法性校验用）。
    pub fn king_in_palace(&self, color: Color) -> bool {
        let k = self.king_index(color);
        let p = self.squares[k as usize];
        if color_of(p) != Some(color) || kind_of(p) != Some(PieceKind::King) {
            return false;
        }
        in_palace(color, col_of(k), row_of(k))
    }

    // ------------------------------------------------------------ 位图维护

    #[inline]
    fn occ_mut(&mut self, color: Color) -> &mut u128 {
        match color {
            Color::Red => &mut self.red_occ,
            Color::Black => &mut self.black_occ,
        }
    }

    /// 某方占据位图（供 `xq-ai` 的攻击检测优化使用）。
    #[inline]
    pub const fn occupancy(&self, color: Color) -> u128 {
        match color {
            Color::Red => self.red_occ,
            Color::Black => self.black_occ,
        }
    }

    /// 双方合起来的占据位图。
    #[inline]
    pub const fn all_occupancy(&self) -> u128 {
        self.red_occ | self.black_occ
    }

    #[inline]
    fn set_occ(&mut self, color: Color, idx: u8) {
        *self.occ_mut(color) |= 1u128 << idx;
    }

    #[inline]
    fn clear_occ(&mut self, color: Color, idx: u8) {
        *self.occ_mut(color) &= !(1u128 << idx);
    }

    /// 从零重算哈希（一致性校验用）。
    fn compute_hash(squares: &[u8; BOARD_SIZE], side: Color) -> u64 {
        let mut h = 0u64;
        for (i, &p) in squares.iter().enumerate() {
            if p != EMPTY {
                h ^= ZOBRIST.pieces[zobrist_index(p)][i];
            }
        }
        if side == Color::Black {
            h ^= ZOBRIST.side;
        }
        h
    }

    // ------------------------------------------------------ make / unmake

    /// 应用一个**调用方已保证合法**的着法，返回可用于回滚的记录。
    ///
    /// 不做任何校验，是搜索内层循环用的快通道。公开 API 请用
    /// [`Position::make_move`]。
    ///
    /// 步骤顺序严格遵循 [docs/03](../../../docs/03-规则引擎与领域模型.md) §7.3：
    /// 位图与将帅缓存必须先于哈希更新完成，否则会出现隐蔽的不一致。
    pub fn make_move_unchecked(&mut self, mv: Move) -> MoveRecord {
        let from = mv.from();
        let to = mv.to();
        let us = self.side_to_move;
        let them = us.opponent();

        let piece = self.squares[from as usize];
        let captured = self.squares[to as usize];

        let record = MoveRecord {
            mv,
            captured,
            mover: us,
            prev_halfmove_clock: self.halfmove_clock,
            prev_fullmove_number: self.fullmove_number,
            prev_hash: self.hash,
            gave_check: false,
        };

        // 2) 吃子：移出棋盘、更新对方位图、半步计数归零
        if captured != EMPTY {
            self.squares[to as usize] = EMPTY;
            self.clear_occ(them, to);
            self.halfmove_clock = 0;
        } else {
            self.halfmove_clock += 1;
        }

        // 4) 移动棋子
        self.squares[from as usize] = EMPTY;
        self.squares[to as usize] = piece;
        self.clear_occ(us, from);
        self.set_occ(us, to);

        // 5) 将帅缓存
        if kind_of(piece) == Some(PieceKind::King) {
            match us {
                Color::Red => self.red_king = to,
                Color::Black => self.black_king = to,
            }
        }

        // 6) Zobrist 增量
        self.hash ^= ZOBRIST.pieces[zobrist_index(piece)][from as usize];
        if captured != EMPTY {
            self.hash ^= ZOBRIST.pieces[zobrist_index(captured)][to as usize];
        }
        self.hash ^= ZOBRIST.pieces[zobrist_index(piece)][to as usize];
        self.hash ^= ZOBRIST.side;

        // 7~8) 走子方与回合数
        if us == Color::Black {
            self.fullmove_number += 1;
        }
        self.side_to_move = them;

        // 9) 判断是否将军（供重复局面裁决与 xq-coach 使用）
        let mut record = record;
        record.gave_check = self.is_attacked(self.king_index(them), us);

        // 10) 历史
        self.history.push(self.hash);
        self.move_stack.push(record);
        record
    }

    /// 回滚最后一步。
    pub fn unmake_move_unchecked(&mut self) -> Option<MoveRecord> {
        let record = self.move_stack.pop()?;
        let mv = record.mv;
        let from = mv.from();
        let to = mv.to();
        let us = record.mover;
        let piece = self.squares[to as usize];

        self.side_to_move = us;
        self.fullmove_number = record.prev_fullmove_number;
        self.halfmove_clock = record.prev_halfmove_clock;

        self.squares[from as usize] = piece;
        self.squares[to as usize] = record.captured;

        self.clear_occ(us, to);
        self.set_occ(us, from);
        if record.captured != EMPTY {
            self.set_occ(us.opponent(), to);
        }

        if kind_of(piece) == Some(PieceKind::King) {
            match us {
                Color::Red => self.red_king = from,
                Color::Black => self.black_king = from,
            }
        }

        self.hash = record.prev_hash;
        self.history.pop();
        Some(record)
    }

    /// 应用一个着法，失败时局面保持原样。
    ///
    /// 这是**权威裁决入口** —— 服务端用它校验客户端提交的着法。
    pub fn make_move(&mut self, mv: Move) -> Result<(), IllegalMove> {
        let from = mv.from();
        let to = mv.to();
        if from == to {
            return Err(IllegalMove::Null);
        }
        if !is_valid_index(from) || !is_valid_index(to) {
            return Err(IllegalMove::OffBoard { from, to });
        }
        let piece = self.squares[from as usize];
        match color_of(piece) {
            None => return Err(IllegalMove::NoPiece { from }),
            Some(c) if c != self.side_to_move => {
                return Err(IllegalMove::WrongSide {
                    from,
                    expected: self.side_to_move,
                });
            }
            Some(_) => {}
        }

        // 完整合法性校验。
        //
        // ⚠️ 只查「走后己方将帅是否被将」是**不够的**：那样会放过
        // 「车穿过己方兵」「马蹩着腿跳过去」这类几何上明显非法的着法。
        // 本函数是对外与**服务端**的权威裁决入口，必须完整。
        //
        // 代价说明：这里会产生一次完整的合法着法列表（约 45 组 make/unmake）。
        // 搜索热点路径**不走这里**，用的是 `make_move_unchecked`，因此没有性能影响。
        if !self.is_legal(mv) {
            return Err(IllegalMove::NotLegal { from, to });
        }

        self.make_move_unchecked(mv);
        self.verify();
        Ok(())
    }

    /// 撤销最后一步（公开入口）。
    pub fn unmake_move(&mut self) -> Option<MoveRecord> {
        self.unmake_move_unchecked()
    }

    // -------------------------------------------------------- 攻击检测

    /// `target` 格是否被 `by` 方攻击。
    ///
    /// 采用**反向探测**（从目标格反查攻击者），比正向生成全部着法快得多。
    ///
    /// > **将帅的统一处理**：本函数把将帅视为「沿同列直线、中间无异子时可攻击
    /// > 对方将帅」，因此「白脸将」被统一为一种攻击关系 —— 这样合法性过滤只需
    /// > 检查一次 `is_attacked(己方将帅格, 对方)`，无需单独写一套照面判定。
    /// > 代价是：非将帅格若与对方将帅同列且中间无子，也会返回 `true`。
    /// > 这在本 crate 内不影响正确性（所有调用点都传将帅格），但**外部若想用它
    /// > 判断「某个子是否被保护」，需要自行排除纵向的将帅情况**。
    pub fn is_attacked(&self, target: u8, by: Color) -> bool {
        debug_assert!(is_valid_index(target));
        let tc = col_of(target) as i8;
        let tr = row_of(target) as i8;

        // ---- 1) 车 / 将帅：四方向射线，遇第一个棋子即判定 ----
        for &(dc, dr) in ORTHO.iter() {
            let mut c = tc + dc;
            let mut r = tr + dr;
            let mut dist = 1i32;
            while on_board(c, r) {
                let p = self.squares[index(c as u8, r as u8) as usize];
                if p != EMPTY {
                    if color_of(p) == Some(by) {
                        match kind_of(p) {
                            Some(PieceKind::Chariot) => return true,
                            // 纵向任意距离（白脸将），或横向相邻一步
                            Some(PieceKind::King) if dc == 0 || dist == 1 => return true,
                            _ => {}
                        }
                    }
                    break;
                }
                c += dc;
                r += dr;
                dist += 1;
            }
        }

        // ---- 2) 炮：跳过第一个棋子（炮架）后，第二个棋子若为敌炮则被攻击 ----
        for &(dc, dr) in ORTHO.iter() {
            let mut c = tc + dc;
            let mut r = tr + dr;
            let mut screen_seen = false;
            while on_board(c, r) {
                let p = self.squares[index(c as u8, r as u8) as usize];
                if p != EMPTY {
                    if !screen_seen {
                        screen_seen = true;
                    } else {
                        if color_of(p) == Some(by) && kind_of(p) == Some(PieceKind::Cannon) {
                            return true;
                        }
                        break;
                    }
                }
                c += dc;
                r += dr;
            }
        }

        // ---- 3) 马：反推 8 个可能的来源格，并检查其蹩腿点是否为空 ----
        for &(dc, dr, lc, lr) in HORSE_ATTACKERS.iter() {
            let hc = tc + dc;
            let hr = tr + dr;
            if !on_board(hc, hr) {
                continue;
            }
            let hp = self.squares[index(hc as u8, hr as u8) as usize];
            if color_of(hp) != Some(by) || kind_of(hp) != Some(PieceKind::Horse) {
                continue;
            }
            let blc = tc + lc;
            let blr = tr + lr;
            if !on_board(blc, blr) {
                continue;
            }
            if self.squares[index(blc as u8, blr as u8) as usize] == EMPTY {
                return true;
            }
        }

        // ---- 4) 兵 / 卒 ----
        // 纵向：兵只能向前吃，故攻击者位于 target - forward
        let pr = tr - by.forward();
        if on_board(tc, pr) {
            let p = self.squares[index(tc as u8, pr as u8) as usize];
            if color_of(p) == Some(by) && kind_of(p) == Some(PieceKind::Pawn) {
                return true;
            }
        }
        // 横向：必须已过河。攻击者位于同行左右相邻格，其所在行即 target 所在行。
        if crate::square::has_crossed_river(by, tr as u8) {
            for dc in [-1i8, 1] {
                let c = tc + dc;
                if !on_board(c, tr) {
                    continue;
                }
                let p = self.squares[index(c as u8, tr as u8) as usize];
                if color_of(p) == Some(by) && kind_of(p) == Some(PieceKind::Pawn) {
                    return true;
                }
            }
        }

        false
    }

    /// `color` 方是否正被将军（含白脸将）。
    #[inline]
    pub fn is_in_check(&self, color: Color) -> bool {
        self.is_attacked(self.king_index(color), color.opponent())
    }

    // ---------------------------------------------------------- 着色移动

    /// 生成全部伪合法着法（不检查走后己方将帅是否被将）。
    #[inline]
    pub fn pseudo_legal_moves(&self) -> MoveList {
        let mut list = MoveList::new();
        self.gen_pseudo_into(&mut list);
        list
    }

    /// 生成全部**合法**着法。
    ///
    /// 用于 UI 高亮 —— 每次调用会分配一个 `Vec`，**不适合搜索内层循环**；
    /// 搜索路径请用 [`Position::has_legal_move`] 或 `gen_legal_into`。
    pub fn legal_moves(&mut self) -> Vec<Move> {
        let mut list = MoveList::new();
        self.gen_legal_into(&mut list);
        list.to_vec()
    }

    /// 生成全部合法着法到调用方提供的列表。
    pub fn gen_legal_into(&mut self, out: &mut MoveList) {
        out.clear();
        let mut pseudo = MoveList::new();
        self.gen_pseudo_into(&mut pseudo);
        let us = self.side_to_move;
        let them = us.opponent();
        for i in 0..pseudo.len() {
            let mv = pseudo.get(i);
            let record = self.make_move_unchecked(mv);
            let legal = !self.is_attacked(self.king_index(us), them);
            self.unmake_move_unchecked();
            debug_assert_eq!(self.hash, record.prev_hash, "unmake 后哈希未还原");
            if legal {
                out.push(mv);
            }
        }
    }

    /// 是否存在至少一个合法着法（短路，找到第一个即返回）。
    ///
    /// 搜索与终局判定专用 —— 不分配内存。
    pub fn has_legal_move(&mut self) -> bool {
        let mut pseudo = MoveList::new();
        self.gen_pseudo_into(&mut pseudo);
        let us = self.side_to_move;
        let them = us.opponent();
        for i in 0..pseudo.len() {
            let mv = pseudo.get(i);
            let _ = self.make_move_unchecked(mv);
            let legal = !self.is_attacked(self.king_index(us), them);
            self.unmake_move_unchecked();
            if legal {
                return true;
            }
        }
        false
    }

    /// 若 `from → to` 是合法着法则返回它。
    ///
    /// UI 交互专用：把「点击起点、点击终点」转成经过校验的 `Move`。
    pub fn legal_move_to(&mut self, from: u8, to: u8) -> Option<Move> {
        if !is_valid_index(from) || !is_valid_index(to) || from == to {
            return None;
        }
        let mv = Move::new(from, to);
        let mut list = MoveList::new();
        self.gen_legal_into(&mut list);
        list.contains(mv).then_some(mv)
    }

    /// 某个着法是否合法。
    pub fn is_legal(&mut self, mv: Move) -> bool {
        let mut list = MoveList::new();
        self.gen_legal_into(&mut list);
        list.contains(mv)
    }

    // ------------------------------------------------------------ 一致性

    /// 一致性校验：检查位图、将帅缓存、哈希、历史栈长度四项，任何一项不符即 panic。
    ///
    /// **方法本身在任何构建下都存在**，便于测试与基准随时调用；但库内部的热点
    /// 路径（[`Position::make_move`]）只在 debug 构建下自动调用它 —— 见 `verify()`，
    /// 这样 release 构建零开销。
    pub fn assert_consistent(&self) {
        let mut red = 0u128;
        let mut black = 0u128;
        for (i, &p) in self.squares.iter().enumerate() {
            match color_of(p) {
                None => {}
                Some(Color::Red) => red |= 1u128 << i,
                Some(Color::Black) => black |= 1u128 << i,
            }
        }
        assert_eq!(red, self.red_occ, "红方占据位图与棋盘不一致");
        assert_eq!(black, self.black_occ, "黑方占据位图与棋盘不一致");

        assert_eq!(
            Self::compute_hash(&self.squares, self.side_to_move),
            self.hash,
            "Zobrist 增量维护错误"
        );

        for c in [Color::Red, Color::Black] {
            let k = self.king_index(c);
            assert!(is_valid_index(k), "{}方将帅缓存越界", c.name_zh());
            let p = self.squares[k as usize];
            assert_eq!(
                kind_of(p),
                Some(PieceKind::King),
                "{}方将帅缓存指向了非将帅棋子",
                c.name_zh()
            );
            assert_eq!(color_of(p), Some(c), "{}方将帅缓存颜色错误", c.name_zh());
        }

        assert_eq!(
            self.history.len(),
            self.move_stack.len() + 1,
            "历史哈希栈长度与着法栈不匹配"
        );
        assert_eq!(
            *self.history.last().expect("history 至少含初始局面"),
            self.hash,
            "历史栈顶哈希与当前哈希不一致"
        );
    }

    /// 在 debug 构建下做一致性校验；release 下为空操作（零开销）。
    #[inline]
    fn verify(&self) {
        #[cfg(debug_assertions)]
        self.assert_consistent();
    }
}

/// 马的反向探测表：`(马的相对列偏移, 马的相对行偏移, 蹩腿点的相对列偏移, 蹩腿点的相对行偏移)`。
///
/// 全部相对**目标格**。马位于 `target + (dc, dr)`，其蹩腿点位于 `target + (lc, lr)`。
///
/// 推导：设马到目标的位移为 `(Δc, Δr) = (-dc, -dr)`。
/// `|Δc|=1, |Δr|=2` 时马腿在 `马 + (0, sign(Δr))`；
/// `|Δc|=2, |Δr|=1` 时马腿在 `马 + (sign(Δc), 0)`。
/// 折算到目标坐标系后，8 个蹩腿点恰好是目标的 4 个斜邻格，各出现两次。
const HORSE_ATTACKERS: [(i8, i8, i8, i8); 8] = [
    (1, 2, 1, 1),
    (1, -2, 1, -1),
    (-1, 2, -1, 1),
    (-1, -2, -1, -1),
    (2, 1, 1, 1),
    (2, -1, 1, -1),
    (-2, 1, -1, 1),
    (-2, -1, -1, -1),
];

/// 编译期护栏：常量表必须与棋盘尺寸自洽。
const _: () = {
    assert!(BOARD_SIZE == 90);
    assert!(COLS == 9);
    assert!(ROWS == 10);
    assert!(MAX_MOVES >= 128);
};

/// 供测试与工具使用的便捷构造：把 ICCS 坐标字符串直接放子。
///
/// 每个元素是 `(颜色, 棋子种类, ICCS 坐标)`，例如
/// `from_pieces(&[(Color::Red, PieceKind::King, "e0")], Color::Red)`。
pub fn position_from_pieces(
    pieces: &[(Color, PieceKind, &str)],
    side_to_move: Color,
) -> Result<Position, PositionError> {
    let mut squares = [EMPTY; BOARD_SIZE];
    for &(color, kind, coord) in pieces {
        let idx =
            crate::square::from_iccs(coord).unwrap_or_else(|| panic!("非法 ICCS 坐标: {coord}"));
        squares[idx as usize] = encode(color, kind);
    }
    Position::from_squares(squares, side_to_move, 0, 1)
}

impl core::fmt::Debug for Position {
    /// 调试输出渲染成棋盘图 —— 测试失败时一眼能看出局面。
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(
            f,
            "Position({} to move, halfmove={}, fullmove={}, hash={:#018x})",
            self.side_to_move.name_zh(),
            self.halfmove_clock,
            self.fullmove_number,
            self.hash
        )?;
        for row in (0..ROWS).rev() {
            write!(f, "  row {row}  ")?;
            for col in 0..COLS {
                let p = self.squares[index(col, row) as usize];
                match (color_of(p), kind_of(p)) {
                    (Some(c), Some(k)) => write!(f, "{}", k.name_zh(c))?,
                    _ => write!(f, "·")?,
                }
                if col + 1 < COLS {
                    write!(f, " ")?;
                }
            }
            writeln!(f)?;
        }
        write!(f, "          a b c d e f g h i")
    }
}
