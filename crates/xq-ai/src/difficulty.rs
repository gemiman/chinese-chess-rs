//! 五档难度。
//!
//! # 核心设计原则：弱难度 ≠ 浅搜索
//!
//! 如果只是降低搜索深度，低难度 AI 会走出「莫名其妙」的棋（贪吃一个兵却丢掉一个车）。
//! 那不像「人类水平有限」，而像「随机数生成器」，会严重损害体验。
//!
//! **正确做法**：低难度 AI 保持**正常的棋力视野**（知道棋子会被吃、知道将军），
//! 但**主动选择次优着法** —— 这样才像「思路正常但算得不够深」的对手。
//!
//! # 用「分差容差」而不是「前 N 名」
//!
//! 容差以**客观棋力损失**为界：L1 最多只损失 80 厘兵（0.8 个兵）。
//! 这既让它可被战胜，又保证不会走出明显荒谬的着法。
//! 若改用「前 5 名随机」，则无法控制实际损失 —— 前 5 名可能全是坏棋。
//!
//! 全部数值来自 [docs/04 §6](../../../docs/04-AI引擎设计.md) 的**设计初始值**，
//! 必须经实战标定（docs/04 §8.2）后修订。

use crate::rng::Rng;
use crate::search::SearchLimits;
use xq_core::Move;

/// 难度档位。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Difficulty {
    /// 入门。
    L1,
    /// 初级。
    L2,
    /// 中级。
    L3,
    /// 高级。
    L4,
    /// 大师。
    L5,
}

impl Difficulty {
    /// 全部档位，由弱到强。
    pub const ALL: [Difficulty; 5] = [
        Difficulty::L1,
        Difficulty::L2,
        Difficulty::L3,
        Difficulty::L4,
        Difficulty::L5,
    ];

    /// 稳定标识（用于存档与协议）。
    pub const fn id(self) -> &'static str {
        match self {
            Difficulty::L1 => "l1",
            Difficulty::L2 => "l2",
            Difficulty::L3 => "l3",
            Difficulty::L4 => "l4",
            Difficulty::L5 => "l5",
        }
    }

    /// 从标识解析。
    pub fn from_id(id: &str) -> Option<Difficulty> {
        Difficulty::ALL.into_iter().find(|d| d.id() == id)
    }

    /// UI 主文案。
    pub const fn label(self) -> &'static str {
        match self {
            Difficulty::L1 => "入门",
            Difficulty::L2 => "初级",
            Difficulty::L3 => "中级",
            Difficulty::L4 => "高级",
            Difficulty::L5 => "大师",
        }
    }

    /// UI 副标题。
    ///
    /// > ⚠️ 这些描述是**设计稿说法**，必须在胜率标定（docs/04 §8.2）完成后
    /// > 依据实测数据修订。标定前不得在正式宣传材料中使用。
    pub const fn subtitle(self) -> &'static str {
        match self {
            Difficulty::L1 => "刚学会走法，适合熟悉规则",
            Difficulty::L2 => "会基本的吃子与防守",
            Difficulty::L3 => "有基本战术意识，会计算几步",
            Difficulty::L4 => "会布局与组合战术，算得较深",
            Difficulty::L5 => "接近业余强手水平",
        }
    }

    /// 该档位的搜索与随机化参数。
    pub fn profile(self) -> DifficultyProfile {
        match self {
            Difficulty::L1 => DifficultyProfile {
                level: self,
                limits: SearchLimits {
                    max_depth: 2,
                    // 一档刻意关闭静态搜索与置换表：棋力更低，也更快
                    use_quiescence: false,
                    use_tt: false,
                    max_nodes: None,
                },
                tolerance: 80,
                weight_decay: 0.6,
                mistake_rate: 0.30,
            },
            Difficulty::L2 => DifficultyProfile {
                level: self,
                limits: SearchLimits {
                    max_depth: 4,
                    use_quiescence: true,
                    use_tt: false,
                    max_nodes: None,
                },
                tolerance: 40,
                weight_decay: 0.7,
                mistake_rate: 0.15,
            },
            Difficulty::L3 => DifficultyProfile {
                level: self,
                limits: SearchLimits {
                    max_depth: 6,
                    use_quiescence: true,
                    use_tt: true,
                    max_nodes: None,
                },
                tolerance: 20,
                weight_decay: 0.8,
                mistake_rate: 0.05,
            },
            Difficulty::L4 => DifficultyProfile {
                level: self,
                limits: SearchLimits {
                    max_depth: 8,
                    use_quiescence: true,
                    use_tt: true,
                    max_nodes: None,
                },
                // 从 L4 起不再随机化：要赢就得走最优着法
                tolerance: 0,
                weight_decay: 1.0,
                mistake_rate: 0.0,
            },
            Difficulty::L5 => DifficultyProfile {
                level: self,
                limits: SearchLimits {
                    // 时间限制由外部 StopSignal 负责（零 IO 约束），这里只放开深度
                    max_depth: u8::MAX,
                    use_quiescence: true,
                    use_tt: true,
                    max_nodes: None,
                },
                tolerance: 0,
                weight_decay: 1.0,
                mistake_rate: 0.0,
            },
        }
    }
}

impl core::fmt::Display for Difficulty {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.label())
    }
}

/// 一个档位的完整参数。
#[derive(Clone, Copy, Debug)]
pub struct DifficultyProfile {
    /// 档位。
    pub level: Difficulty,
    /// 搜索限制。
    pub limits: SearchLimits,
    /// 分差容差（厘兵）。只保留与最优着法分差在此范围内的着法。
    pub tolerance: i32,
    /// 权重衰减系数（按排名指数衰减）。
    pub weight_decay: f64,
    /// 失误率：以该概率故意不选最优着法。
    pub mistake_rate: f64,
}

impl DifficultyProfile {
    /// 是否启用随机化。
    pub fn randomizes(&self) -> bool {
        self.tolerance > 0 && self.mistake_rate > 0.0
    }

    /// 从根着法评分中挑一个着法。
    ///
    /// `root_moves` 必须已按评分**降序**排列（这是 [`crate::Searcher`] 的输出约定）。
    pub fn choose(&self, rng: &mut Rng, root_moves: &[(Move, i32)]) -> Option<Move> {
        let (best_move, best_score) = *root_moves.first()?;

        if !self.randomizes() || root_moves.len() < 2 {
            return Some(best_move);
        }

        // 候选：最优着法之后、且分差在容差内的着法。
        //
        // 注意 `root_moves` 中未被提升 alpha 的着法只得**上界**，其分差被低估，
        // 因此这里的过滤可能把「实际差很多」的着法误当成候选。对「挑选接近最优的
        // 次优着法」这个用途足够；若要精确排序，需在根节点对候选做完整窗口重搜。
        let candidates: Vec<(Move, i32)> = root_moves
            .iter()
            .skip(1)
            .filter(|(_, score)| best_score - score <= self.tolerance)
            .copied()
            .collect();

        if candidates.is_empty() {
            // 容差内没有别的着法（例如只剩一步必走着法）→ 只能走最优
            return Some(best_move);
        }

        // 不失误就直接走最优
        if rng.next_unit() >= self.mistake_rate {
            return Some(best_move);
        }

        // 按权重衰减挑选：越靠前的次优着法越可能被选中
        let weights: Vec<f64> = (0..candidates.len())
            .map(|i| self.weight_decay.powi(i as i32))
            .collect();
        let total: f64 = weights.iter().sum();
        if total <= 0.0 {
            return Some(candidates[0].0);
        }

        let mut roll = rng.next_unit() * total;
        for (i, w) in weights.iter().enumerate() {
            roll -= w;
            if roll <= 0.0 {
                return Some(candidates[i].0);
            }
        }
        Some(candidates[candidates.len() - 1].0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xq_core::Move;

    fn mv(from: u8, to: u8) -> Move {
        Move::new(from, to)
    }

    #[test]
    fn levels_are_ordered_by_strength() {
        let l1 = Difficulty::L1.profile();
        let l5 = Difficulty::L5.profile();
        assert!(l1.limits.max_depth < l5.limits.max_depth);
        assert!(!l1.limits.use_quiescence && l5.limits.use_quiescence);
        assert!(l1.tolerance > l5.tolerance);
        assert!(l1.mistake_rate > l5.mistake_rate);
    }

    #[test]
    fn only_low_levels_randomize() {
        assert!(Difficulty::L1.profile().randomizes());
        assert!(Difficulty::L2.profile().randomizes());
        assert!(Difficulty::L3.profile().randomizes());
        assert!(!Difficulty::L4.profile().randomizes());
        assert!(!Difficulty::L5.profile().randomizes());
    }

    #[test]
    fn id_roundtrip() {
        for level in Difficulty::ALL {
            assert_eq!(Difficulty::from_id(level.id()), Some(level));
        }
        assert_eq!(Difficulty::from_id("l9"), None);
    }

    /// 不随机化的档位必须永远返回最优着法。
    #[test]
    fn strong_levels_always_pick_best() {
        let mut rng = Rng::new(1);
        let roots = vec![(mv(0, 1), 100), (mv(0, 2), 90), (mv(0, 3), 80)];
        for level in [Difficulty::L4, Difficulty::L5] {
            let profile = level.profile();
            for _ in 0..200 {
                assert_eq!(profile.choose(&mut rng, &roots), Some(mv(0, 1)));
            }
        }
    }

    /// 弱档位不能选出容差之外的着法。
    #[test]
    fn weak_levels_respect_tolerance() {
        let mut rng = Rng::new(7);
        let profile = Difficulty::L1.profile(); // 容差 80
        // 次优着法只差 10，第三个差 500（超出容差）
        let roots = vec![(mv(0, 1), 100), (mv(0, 2), 90), (mv(0, 3), -400)];
        for _ in 0..2000 {
            let chosen = profile.choose(&mut rng, &roots).unwrap();
            assert_ne!(chosen, mv(0, 3), "不应选出容差之外的着法");
        }
    }

    /// 只有一步可走时必须返回它（不能因为「失误」返回 None）。
    #[test]
    fn single_move_is_always_returned() {
        let mut rng = Rng::new(3);
        let profile = Difficulty::L1.profile();
        let roots = vec![(mv(0, 1), -500)];
        for _ in 0..100 {
            assert_eq!(profile.choose(&mut rng, &roots), Some(mv(0, 1)));
        }
    }

    /// 空着法列表返回 None。
    #[test]
    fn empty_root_returns_none() {
        let mut rng = Rng::new(4);
        assert_eq!(Difficulty::L5.profile().choose(&mut rng, &[]), None);
    }

    /// 低难度确实会「失误」：在大量采样中应当选到过次优着法。
    #[test]
    fn weak_level_actually_makes_mistakes() {
        let mut rng = Rng::new(11);
        let profile = Difficulty::L1.profile();
        let roots = vec![(mv(0, 1), 100), (mv(0, 2), 95)];
        let mut suboptimal = 0;
        for _ in 0..2000 {
            if profile.choose(&mut rng, &roots) == Some(mv(0, 2)) {
                suboptimal += 1;
            }
        }
        assert!(
            suboptimal > 100,
            "L1 应当偶尔走次优着法，实际只出现 {suboptimal}/2000 次"
        );
        // 但也不该喧宾夺主 —— 最优着法仍应是多数
        assert!(
            suboptimal < 1000,
            "次优着法出现得过于频繁：{suboptimal}/2000"
        );
    }

    /// 相同种子 → 相同选择序列（可复现性）。
    #[test]
    fn choice_is_reproducible() {
        let roots = vec![(mv(0, 1), 100), (mv(0, 2), 95), (mv(0, 3), 85)];
        let profile = Difficulty::L1.profile();

        let mut a = Rng::new(2024);
        let mut b = Rng::new(2024);
        for _ in 0..500 {
            assert_eq!(
                profile.choose(&mut a, &roots),
                profile.choose(&mut b, &roots)
            );
        }
    }
}
