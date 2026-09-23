//! 确定性随机源。
//!
//! # 为什么不用 `rand`
//!
//! [docs/02 §3.1](../../../docs/02-系统架构设计.md) 的依赖规则 R1 规定领域层
//! crate 不引入 IO / 运行时依赖；本实现更进一步，连 `rand` 也不引。理由是这里的
//! 需求极其简单 —— **给定种子、结果可复现** —— 而 SplitMix64 十几行就能满足，
//! 且带来两个额外好处：
//!
//! 1. `xq-ai` 保持**零运行时依赖**；
//! 2. 「不引入系统随机」由代码结构保证，而不是靠约定与自律。
//!
//! 若将来需要更复杂的随机分布（正态、洗牌算法），再考虑引入 `rand` 并附带
//! 显式种子注入。

/// SplitMix64 状态。
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// 用给定种子创建。相同种子必然产生相同序列。
    pub const fn new(seed: u64) -> Self {
        // 先混一次，避免 0/1/2 这类小种子产生相关的起始状态
        Self {
            state: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x1234_5678_9ABC_DEF0,
        }
    }

    /// 下一个 64 位随机数。
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `0..bound` 内的均匀整数。`bound == 0` 时返回 0。
    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }

    /// `[0, 1)` 的浮点数。用于「按权重挑选」。
    pub fn next_unit(&mut self) -> f64 {
        // 取高 53 位，落到 [0,1)
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seed_different_sequence() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn below_respects_bound() {
        let mut rng = Rng::new(7);
        for _ in 0..10_000 {
            assert!(rng.below(12) < 12);
        }
        assert_eq!(rng.below(0), 0);
    }

    #[test]
    fn next_unit_in_range() {
        let mut rng = Rng::new(99);
        for _ in 0..10_000 {
            let v = rng.next_unit();
            assert!((0.0..1.0).contains(&v), "{v} 越界");
        }
    }
}
