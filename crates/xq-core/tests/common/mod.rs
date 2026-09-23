//! 集成测试共用的工具。
//!
//! 自带确定性 PRNG —— 测试失败时可以凭 seed 精确复现，不依赖系统熵。

#![allow(dead_code)]

/// SplitMix64 驱动的测试随机源。给定同一 seed，序列必然一致。
pub struct TestRng(u64);

impl TestRng {
    /// 新建。
    pub const fn new(seed: u64) -> Self {
        // 先混一次，避免 seed=0/1/2 这类小值产生相关的起始状态
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x1234_5678_9ABC_DEF0)
    }

    /// 下一个 u64。
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `0..bound` 内的均匀整数。`bound` 为 0 时返回 0。
    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }
}
