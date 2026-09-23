//! Zobrist 哈希表。
//!
//! # 为什么必须固定种子
//!
//! 哈希必须**跨会话稳定**，原因有三：
//!
//! 1. 置换表（TT）跨对局复用时，同一局面必须得到同一哈希；
//! 2. 开局库 / 残局库以哈希为键；
//! 3. 测试需要可复现。
//!
//! 因此本实现用**编译期常量种子**驱动内置的 SplitMix64，全程不触碰系统熵。
//!
//! # 与设计文档的差异
//!
//! [docs/03](../../../docs/03-规则引擎与领域模型.md) §7.4 建议用
//! `StdRng::seed_from_u64(0x9E3779B97F4A7C15)`。本实现改为自带 SplitMix64，
//! 因为 [docs/02](../../../docs/02-系统架构设计.md) §3.1 的依赖规则 R1 规定
//! `xq-core` **不依赖任何内部 crate，仅依赖 `serde`（可选）与零成本工具**——
//! 引入 `rand` 会违反该条。自带 PRNG 有两个额外好处：
//!
//! - `xq-core` 保持**零运行时依赖**；
//! - 「不引入系统随机」由**代码结构**保证，而不是靠约定（不存在 `thread_rng` 可调用）。
//!
//! 种子值与文档建议值保持一致，语义完全等价。

use crate::square::BOARD_SIZE;

/// 与设计文档一致的种子常量（黄金比例倒数的高 64 位）。
const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// SplitMix64：Steele 等人提出的简洁可分割随机数生成器。
///
/// 状态自增一个奇数常量后做三轮雪崩混合。周期 2^64，统计质量足够做哈希表，
/// 且是**纯函数式**的 —— 给定同一状态序列，输出必然一致。
#[inline]
const fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Zobrist 表。14 种棋子（红 7 + 黑 7）× 90 格，外加一个走子方键。
pub struct Zobrist {
    /// `[棋子索引][格子索引]`，棋子索引见 [`crate::piece::zobrist_index`]。
    pub pieces: [[u64; BOARD_SIZE]; 14],
    /// 走子方为黑时异或此键。
    pub side: u64,
}

impl Zobrist {
    /// 编译期构造哈希表。
    const fn new() -> Self {
        let mut state = SEED;
        let mut pieces = [[0u64; BOARD_SIZE]; 14];
        let mut p = 0;
        while p < 14 {
            let mut s = 0;
            while s < BOARD_SIZE {
                pieces[p][s] = splitmix64(&mut state);
                s += 1;
            }
            p += 1;
        }
        // 最后一个键给走子方，保持与「先棋子后走子方」的顺序一致。
        let side = splitmix64(&mut state);
        Self { pieces, side }
    }
}

/// 全局唯一的 Zobrist 表。编译期求值，运行时零初始化开销。
pub static ZOBRIST: Zobrist = Zobrist::new();

impl core::fmt::Debug for Zobrist {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // 1261 个 u64 全打印出来没有意义，只暴露规模与走子方键。
        f.debug_struct("Zobrist")
            .field("pieces", &"[[u64; 90]; 14]")
            .field("side", &format_args!("{:#018x}", self.side))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 1261 个键必须互不相同 —— 只要有重复，就会产生系统性的哈希碰撞。
    #[test]
    fn all_keys_are_distinct() {
        let mut seen = HashSet::with_capacity(14 * BOARD_SIZE + 1);
        for row in ZOBRIST.pieces.iter() {
            for &k in row.iter() {
                assert!(seen.insert(k), "Zobrist 键重复: {k:#x}");
            }
        }
        assert!(seen.insert(ZOBRIST.side), "side 键与棋子键重复");
        assert_eq!(seen.len(), 14 * BOARD_SIZE + 1);
    }

    /// 键不能退化成 0（异或 0 相当于不更新）。
    #[test]
    fn no_zero_keys() {
        for row in ZOBRIST.pieces.iter() {
            for &k in row.iter() {
                assert_ne!(k, 0);
            }
        }
        assert_ne!(ZOBRIST.side, 0);
    }

    /// 高/低 32 位都应有良好的位分布（粗略卡方式检查）。
    #[test]
    fn bit_distribution_is_reasonable() {
        let total = 14 * BOARD_SIZE;
        let mut high_ones = 0usize;
        let mut low_ones = 0usize;
        for row in ZOBRIST.pieces.iter() {
            for &k in row.iter() {
                high_ones += (k >> 32).count_ones() as usize;
                low_ones += (k & 0xFFFF_FFFF).count_ones() as usize;
            }
        }
        // 期望各占一半，允许 ±5% 偏差
        for ones in [high_ones, low_ones] {
            let expect = total * 32 / 2;
            let lo = expect - expect / 20;
            let hi = expect + expect / 20;
            assert!(
                (lo..=hi).contains(&ones),
                "位分布异常: {ones} 不在 {lo}..={hi}"
            );
        }
    }
}
