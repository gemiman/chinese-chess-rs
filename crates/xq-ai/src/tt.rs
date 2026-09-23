//! 置换表（Transposition Table）。
//!
//! # 作用
//!
//! 不同着法顺序可能到达同一局面（置换）。缓存已搜索结果可避免重复计算 ——
//! 开局与中局的置换率通常达 20~40%。
//!
//! # ⚠️ 最容易写错的地方：杀棋分数的距离调整
//!
//! TT 里存的分数是**该节点视角**的，而搜索里的杀棋分数 `MATE_SCORE - n` 里的
//! `n` 是**从根节点算起的半步数**。同一个局面可能在不同深度被访问到，若不做
//! 调整，就会得到错误的杀棋距离 —— 表现为引擎「看得见杀棋但走错顺序」。
//!
//! 本模块的处理：
//!
//! - 存入时把「根相对」的杀棋分转成「节点相对」：正向杀棋 `+ply`，被杀 `-ply`；
//! - 取出时反向转换。
//!
//! 这个 bug 极其隐蔽（普通局面下完全看不出来），所以有专门的测试锁定它。

use xq_core::Move;

/// 精确值（该分数就是真实值）。
pub const FLAG_EXACT: u8 = 0;
/// 下界（发生了 beta 截断，真实值 ≥ 该分数）。
pub const FLAG_LOWER: u8 = 1;
/// 上界（alpha 未改进，真实值 ≤ 该分数）。
pub const FLAG_UPPER: u8 = 2;

/// 一条置换表记录。
///
/// 严格 16 字节，与缓存行对齐友好。用 32 位 key 做校验（而不是存完整 64 位哈希）
/// 是内存与误命中率的折衷：误命中概率约 `2^-32`，对单次搜索而言可忽略。
#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct TtEntry {
    /// 哈希高 32 位，用于校验。
    key: u32,
    /// 最佳着法的 `Move::0` 原始编码。
    best: u16,
    /// 评分（厘兵，节点相对）。
    score: i16,
    /// 搜索深度。
    depth: i8,
    /// FLAG_EXACT / FLAG_LOWER / FLAG_UPPER
    flag: u8,
    /// 世代号，用于替换策略。
    generation: u8,
    _padding: [u8; 2],
}

impl TtEntry {
    const EMPTY: TtEntry = TtEntry {
        key: 0,
        best: 0,
        score: 0,
        depth: -1,
        flag: FLAG_EXACT,
        generation: 0,
        _padding: [0; 2],
    };
}

/// 一次命中的结果。
#[derive(Clone, Copy, Debug)]
pub struct TtProbe {
    /// 评分（已从节点相对换算回根相对）。
    pub score: i32,
    /// 该条目对应的搜索深度。
    pub depth: i8,
    /// FLAG_*
    pub flag: u8,
    /// 该局面上次搜出的最佳着法。
    pub best: Option<Move>,
}

/// 置换表。
pub struct TranspositionTable {
    entries: Vec<TtEntry>,
    mask: usize,
    generation: u8,
    /// 命中统计（供基准测试与调试）。
    hits: u64,
    probes: u64,
}

impl TranspositionTable {
    /// 按目标内存大小创建（内部会向下取整到 2 的幂）。
    pub fn new(size_mb: usize) -> Self {
        let bytes = size_mb.max(1) * 1024 * 1024;
        let want = (bytes / core::mem::size_of::<TtEntry>()).max(1024);
        // 向下取到 2 的幂，这样索引可以用位与代替取模
        let count = 1usize << (usize::BITS - 1 - want.leading_zeros());
        Self {
            entries: vec![TtEntry::EMPTY; count],
            mask: count - 1,
            generation: 1,
            hits: 0,
            probes: 0,
        }
    }

    /// 条目数（2 的幂）。
    pub fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// 清空。换局时调用，避免上一局的陈旧数据干扰。
    pub fn clear(&mut self) {
        for entry in self.entries.iter_mut() {
            *entry = TtEntry::EMPTY;
        }
        self.generation = 1;
        self.hits = 0;
        self.probes = 0;
    }

    /// 进入新一代。旧世代条目在冲突时优先被替换。
    pub fn new_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            // 世代号回绕：清空重来，避免 0 与「空条目」语义冲突
            self.clear();
        }
    }

    /// 命中率（0.0 ~ 1.0）。
    pub fn hit_rate(&self) -> f64 {
        if self.probes == 0 {
            0.0
        } else {
            self.hits as f64 / self.probes as f64
        }
    }

    #[inline]
    fn index_of(&self, hash: u64) -> usize {
        (hash as usize) & self.mask
    }

    /// 查询。
    pub fn probe(&mut self, hash: u64, ply: i32) -> Option<TtProbe> {
        let entry = self.entries[self.index_of(hash)];
        self.probes += 1;
        if entry.depth < 0 || entry.key != (hash >> 32) as u32 {
            return None;
        }
        self.hits += 1;
        Some(TtProbe {
            score: score_from_tt(entry.score as i32, ply),
            depth: entry.depth,
            flag: entry.flag,
            best: (entry.best != 0).then_some(Move(entry.best)),
        })
    }

    /// 只取最佳着法（不做深度判断，用于着法排序）。
    pub fn probe_move(&self, hash: u64) -> Option<Move> {
        let entry = self.entries[self.index_of(hash)];
        if entry.depth < 0 || entry.key != (hash >> 32) as u32 || entry.best == 0 {
            return None;
        }
        Some(Move(entry.best))
    }

    /// 写入。
    ///
    /// 替换策略：**深度更深者优先；同深度时新世代优先**。
    /// 这样既保证深层结果（更有价值）不被浅层覆盖，又能防止旧世代条目长期占位。
    pub fn store(
        &mut self,
        hash: u64,
        depth: i8,
        score: i32,
        flag: u8,
        best: Option<Move>,
        ply: i32,
    ) {
        // 评分必须落进 i16 —— 超出说明搜索里出现了未预期的极值
        debug_assert!(
            score.abs() < i16::MAX as i32,
            "TT 分数 {score} 超出 i16 表示范围"
        );
        let idx = self.index_of(hash);
        let existing = self.entries[idx];

        let key = (hash >> 32) as u32;
        let same_position = existing.key == key && existing.depth >= 0;
        let should_replace = if !same_position {
            // 位置不同：深度更深，或世代更新
            depth >= existing.depth || existing.generation != self.generation
        } else {
            // 同一位置：只有更深才覆盖，避免用浅层结果冲掉深层结果
            depth >= existing.depth
        };

        if !should_replace {
            return;
        }

        self.entries[idx] = TtEntry {
            key,
            best: best.map(|m| m.0).unwrap_or(0),
            score: score_to_tt(score, ply) as i16,
            depth,
            flag,
            generation: self.generation,
            _padding: [0; 2],
        };
    }
}

impl Default for TranspositionTable {
    fn default() -> Self {
        Self::new(DEFAULT_TT_MB)
    }
}

impl core::fmt::Debug for TranspositionTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // 不打印条目内容（几百万条没有意义），只暴露容量与统计
        f.debug_struct("TranspositionTable")
            .field("capacity", &self.entries.len())
            .field("generation", &self.generation)
            .field("hit_rate", &self.hit_rate())
            .finish()
    }
}

/// 默认置换表大小（MB）。客户端默认档，见 docs/04 §3.4 的容量规划。
pub const DEFAULT_TT_MB: usize = 32;

/// 节点相对的杀棋分 → 根相对的杀棋分。
///
/// 「节点相对」= 该分数是站在**当前节点**看到的；「根相对」= 站在根节点看到的。
/// 二者相差 `ply` 个半步。
#[inline]
pub fn score_from_tt(score: i32, ply: i32) -> i32 {
    if score >= crate::MATE_THRESHOLD {
        score - ply
    } else if score <= -crate::MATE_THRESHOLD {
        score + ply
    } else {
        score
    }
}

/// 根相对的杀棋分 → 节点相对的杀棋分。见 [`score_from_tt`]。
#[inline]
pub fn score_to_tt(score: i32, ply: i32) -> i32 {
    if score >= crate::MATE_THRESHOLD {
        score + ply
    } else if score <= -crate::MATE_THRESHOLD {
        score - ply
    } else {
        score
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_is_sixteen_bytes() {
        assert_eq!(
            core::mem::size_of::<TtEntry>(),
            16,
            "置换表条目必须是 16 字节（缓存行友好）"
        );
    }

    #[test]
    fn capacity_is_power_of_two() {
        for mb in [1, 2, 4, 8, 16, 32] {
            let tt = TranspositionTable::new(mb);
            assert!(tt.capacity().is_power_of_two());
            assert!(tt.capacity() >= 1024);
        }
    }

    #[test]
    fn store_then_probe_roundtrip() {
        let mut tt = TranspositionTable::new(1);
        let hash = 0xDEAD_BEEF_CAFE_BABE_u64;
        let mv = Move::new(25, 22);

        assert!(tt.probe(hash, 0).is_none(), "空表不应命中");
        tt.store(hash, 6, 123, FLAG_EXACT, Some(mv), 0);

        let probe = tt.probe(hash, 0).expect("刚写入的条目应能命中");
        assert_eq!(probe.score, 123);
        assert_eq!(probe.depth, 6);
        assert_eq!(probe.best, Some(mv));
        assert_eq!(tt.probe_move(hash), Some(mv));
    }

    #[test]
    fn different_hash_misses() {
        let mut tt = TranspositionTable::new(1);
        tt.store(0x1111_2222_3333_4444, 5, 50, FLAG_EXACT, None, 0);
        assert!(tt.probe(0x9999_8888_7777_6666, 0).is_none());
    }

    /// 杀棋分数的距离调整必须严格互逆。
    #[test]
    fn mate_score_adjustment_is_inverse() {
        for ply in 0..30 {
            for dist in 1..20 {
                let node_score = crate::MATE_SCORE - dist;
                assert_eq!(score_from_tt(score_to_tt(node_score, ply), ply), node_score);
                let neg = -(crate::MATE_SCORE - dist);
                assert_eq!(score_from_tt(score_to_tt(neg, ply), ply), neg);
            }
        }
        // 普通分数不受影响
        for ply in 0..30 {
            assert_eq!(score_from_tt(score_to_tt(77, ply), ply), 77);
            assert_eq!(score_from_tt(score_to_tt(-321, ply), ply), -321);
        }
    }

    /// 同一局面在**不同 ply** 存取，必须还原出各自的根相对分数。
    ///
    /// 推导（以正向杀棋为例）：
    ///
    /// - 搜索返回的分数是**根相对**的：`MATE - n` 表示「从根算起 n 个半步后成杀」；
    /// - 而一个局面自身的杀棋距离是**位置固有**的，与它是被第几层走到的无关；
    /// - 所以在 ply 4 得到 `MATE - 6`（根距离 6，位置距离 6-4=2）时，
    ///   存进表里的应是位置相对的 `MATE - 2`；
    /// - 同一个位置若在 ply 2 被访问到，还原出的根相对分应是 `MATE - 4`。
    ///
    /// 不做这个换算，引擎就会「看得见杀棋但走错顺序」。
    #[test]
    fn same_position_at_different_ply_yields_correct_root_score() {
        let root_at_ply4 = crate::MATE_SCORE - 6; // 根距离 6，位置距离 2
        let stored = score_to_tt(root_at_ply4, 4);
        assert_eq!(
            stored,
            crate::MATE_SCORE - 2,
            "存入的应是位置相对分（位置距离 2）"
        );

        // 同一个位置：在 ply 4 取出应还原原值
        assert_eq!(score_from_tt(stored, 4), root_at_ply4);
        // 在 ply 2 取出 → 根距离变成 2 + 2 = 4
        assert_eq!(score_from_tt(stored, 2), crate::MATE_SCORE - 4);
        // 在 ply 0 取出 → 根距离就是位置距离 2
        assert_eq!(score_from_tt(stored, 0), crate::MATE_SCORE - 2);
    }

    /// 被杀（负向杀棋分）的换算方向必须与正向镜像对称。
    #[test]
    fn negative_mate_score_adjustment_is_symmetric() {
        let root_at_ply3 = -(crate::MATE_SCORE - 5);
        let stored = score_to_tt(root_at_ply3, 3);
        assert_eq!(stored, -(crate::MATE_SCORE - 2));
        assert_eq!(score_from_tt(stored, 3), root_at_ply3);
        assert_eq!(score_from_tt(stored, 0), -(crate::MATE_SCORE - 2));
    }

    #[test]
    fn deeper_entry_survives_shallower_write() {
        let mut tt = TranspositionTable::new(1);
        let hash = 0xABCD_1234_5678_9ABC;
        tt.store(hash, 8, 500, FLAG_EXACT, None, 0);
        tt.store(hash, 3, 100, FLAG_EXACT, None, 0);
        assert_eq!(tt.probe(hash, 0).unwrap().depth, 8, "浅层不应覆盖深层");
    }

    #[test]
    fn clear_resets_everything() {
        let mut tt = TranspositionTable::new(1);
        tt.store(42, 5, 10, FLAG_EXACT, None, 0);
        assert!(tt.probe(42, 0).is_some());
        tt.clear();
        assert!(tt.probe(42, 0).is_none());
        assert_eq!(tt.hit_rate(), 0.0);
    }
}
