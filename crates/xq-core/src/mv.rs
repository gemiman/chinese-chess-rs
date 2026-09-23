//! 着法编码与着法列表。
//!
//! 中国象棋**无升变、无王车易位、无吃过路兵**，因此一个着法用 `from` / `to`
//! 两个 8 位字段即可完全表达 —— 这是相比国际象棋着法编码的巨大简化，
//! **不需要** promotion / castling 标志位。

use crate::color::Color;

/// 上限：单个局面的合法着法数。
///
/// 中国象棋单局面合法着法的理论上限约 110 余手，256 有足够余量。
/// 用固定容量数组而非 `Vec`，避免在搜索内层循环中反复分配。
pub const MAX_MOVES: usize = 256;

/// 一个着法：`bit 15..8 = from`，`bit 7..0 = to`（均为棋盘一维索引）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Move(pub u16);

impl Move {
    /// 由起止格子构造。
    #[inline]
    pub const fn new(from: u8, to: u8) -> Self {
        Move(((from as u16) << 8) | to as u16)
    }

    /// 起点索引。
    #[inline]
    pub const fn from(self) -> u8 {
        (self.0 >> 8) as u8
    }

    /// 终点索引。
    #[inline]
    pub const fn to(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    /// 是否为「原地不动」的占位值（仅用于数组初始化，不是合法着法）。
    #[inline]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}

impl core::fmt::Display for Move {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}{}",
            crate::square::to_iccs(self.from()),
            crate::square::to_iccs(self.to())
        )
    }
}

/// 固定容量的着法列表。
#[derive(Clone)]
pub struct MoveList {
    buf: [Move; MAX_MOVES],
    len: usize,
}

impl MoveList {
    /// 新建空列表。
    pub const fn new() -> Self {
        Self {
            buf: [Move(0); MAX_MOVES],
            len: 0,
        }
    }

    /// 追加一个着法。
    #[inline]
    pub fn push(&mut self, mv: Move) {
        debug_assert!(self.len < MAX_MOVES, "着法数超出 MAX_MOVES={MAX_MOVES}");
        if self.len < MAX_MOVES {
            self.buf[self.len] = mv;
            self.len += 1;
        }
    }

    /// 元素个数。
    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// 是否为空。
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 清空（保留容量，供复用）。
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// 按位取着法。越界 panic。
    #[inline]
    pub fn get(&self, i: usize) -> Move {
        self.buf[i]
    }

    /// 视图切片。
    #[inline]
    pub fn as_slice(&self) -> &[Move] {
        &self.buf[..self.len]
    }

    /// 迭代器。
    pub fn iter(&self) -> impl Iterator<Item = Move> + '_ {
        self.as_slice().iter().copied()
    }

    /// 是否包含某着法。
    #[inline]
    pub fn contains(&self, mv: Move) -> bool {
        self.as_slice().contains(&mv)
    }

    /// 转成 `Vec`（仅用于对外 API，搜索路径不应调用）。
    pub fn to_vec(&self) -> Vec<Move> {
        self.as_slice().to_vec()
    }
}

impl Default for MoveList {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for MoveList {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// 着法历史记录，供 `unmake_move` 精确回滚。
///
/// > 与设计文档 [docs/03](../../../docs/03-规则引擎与领域模型.md) §7.2 的差异：
/// > 本实现额外保存 `mover` 与 `prev_fullmove_number` 两个字段。文档标注的字段
/// > 不足以完整回滚 —— 回合数在 `unmake` 时需要还原，而「走子方」在撤销后无法
/// > 反推（只能靠推断，不如直接存）。两者都不增加结构体大小（仍为 16 字节，
/// > 落在对齐填充内）。
#[derive(Clone, Copy, Debug)]
pub struct MoveRecord {
    /// 本步着法。
    pub mv: Move,
    /// 被吃掉的棋子（`EMPTY` 表示未吃子）。
    pub captured: u8,
    /// 走子方。
    pub mover: Color,
    /// 走子前的半步计数。
    pub prev_halfmove_clock: u16,
    /// 走子前的完整回合数。
    pub prev_fullmove_number: u16,
    /// 走子前的 Zobrist 哈希（用于回滚）。
    pub prev_hash: u64,
    /// 本步走完后是否将军了对方。供重复局面裁决（长将）与 `xq-coach` 复用。
    pub gave_check: bool,
}
