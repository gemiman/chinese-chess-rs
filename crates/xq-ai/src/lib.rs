//! # xq-ai —— 中国象棋搜索引擎
//!
//! 迭代加深、PVS、静态搜索、置换表、着法排序、五档难度。**零 IO、零外部依赖**。
//!
//! ## 设计约束
//!
//! 遵守不变量 **I-1（领域层零 IO）**：不读时钟、不读文件、不联网。
//!
//! **时间控制怎么做到零 IO**：不在引擎里读时钟，而是由调用方注入
//! [`StopSignal`]。引擎每搜索 1024 个节点问一次「该停了吗」。
//! 这个设计的副产品是**测试友好** —— 用 [`NodeLimitedStop`] 时，
//! 同一局面 + 同一节点上限在任何机器上都得到完全相同的结果，
//! 于是「引擎是变强了还是变弱了」可以客观判断。
//!
//! **随机性怎么做到零 IO**：难度随机化需要「可复现的随机」，
//! 内置 [`Rng`]（SplitMix64 + 显式种子）即可满足，不引入 `rand`。
//!
//! ## 快速上手
//!
//! ```no_run
//! use xq_ai::{Difficulty, Engine, NeverStop, SearchLimits};
//! use xq_core::Position;
//!
//! let mut engine = Engine::new(2024);
//! engine.set_difficulty(Difficulty::L3);
//!
//! let mut pos = Position::startpos();
//! let result = engine.search(&mut pos, &NeverStop);
//!
//! println!("推荐着法：{:?}", result.best_move);
//! println!("评分：{} 厘兵，深度 {}，节点 {}", result.score, result.depth, result.nodes);
//! ```
//!
//! ## 根着法评分（`root_moves`）—— 一个字段服务三个需求
//!
//! [`SearchResult::root_moves`] 给出**根节点全部着法及其评分**，它同时是：
//!
//! 1. **难度随机化**的输入（筛选容差范围内的候选）；
//! 2. **走棋提示**的数据源（推荐前几着）；
//! 3. **战法讲解**的评价依据（实际着法相对最优着法的分差是定级的唯一依据）。
//!
//! 因此这不是可选字段 —— 搜索**不能在根节点做跨着法剪枝**。
//!
//! ## 刻意未实现的部分
//!
//! | 能力 | 现状 | 原因 |
//! |---|---|---|
//! | 空着剪枝 | 未实现，用 **LMR** 替代 | 需要 `Position` 支持「走空着」，会破坏其 `history`/`move_stack` 长度不变量。为搜索优化动规则内核的公开契约不划算 |
//! | 开局库 | 未实现 | docs/04 §7.3 明确要求数据**授权必须核实**；在没有可用授权的棋谱前，塞 10 条定式只会让 L5 每局开局雷同 |
//! | 增量评估 | 未实现，每叶节点全量评估 | docs/04 §4.5 的刻意延后：先用全量建立正确性基线 |
//! | 搜索内重复局面检测 | 只做 60 回合限着判和 | 长将/长捉由对局层在搜索之外裁决 |

#![deny(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod difficulty;
pub mod eval;
pub mod ordering;
pub mod rng;
pub mod search;
pub mod time;
pub mod tt;

pub use difficulty::{Difficulty, DifficultyProfile};
pub use rng::Rng;
pub use search::{INF, MATE_SCORE, MATE_THRESHOLD, MAX_PLY, SearchLimits, SearchResult, Searcher};
pub use time::{AtomicStop, NeverStop, NodeLimitedStop, StopSignal};
pub use tt::TranspositionTable;

use xq_core::{Move, Position};

/// 引擎配置。
#[derive(Clone, Copy, Debug)]
pub struct EngineConfig {
    /// 随机种子。相同种子 + 相同输入 = 相同结果。
    pub seed: u64,
    /// 置换表大小（MB）。
    pub tt_mb: usize,
    /// 初始难度。
    pub difficulty: Difficulty,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            tt_mb: DEFAULT_TT_MB,
            difficulty: Difficulty::L3,
        }
    }
}

/// 默认置换表大小（MB）。
///
/// [docs/04 §3.4](../../../docs/04-AI引擎设计.md) 按难度分档规划了 16/64/128 MB，
/// 本实现统一用 32 MB —— 分档的真正收益要等实测耗时数据出来才能判断，
/// 现在按档分配只会让 L1 也吃掉 16 MB 内存。
pub const DEFAULT_TT_MB: usize = 32;

/// 搜索引擎门面。
pub struct Engine {
    searcher: Searcher,
    difficulty: Difficulty,
    rng: Rng,
}

impl Engine {
    /// 用种子创建。
    pub fn new(seed: u64) -> Self {
        Self::with_config(EngineConfig {
            seed,
            ..Default::default()
        })
    }

    /// 用完整配置创建。
    pub fn with_config(config: EngineConfig) -> Self {
        Self {
            searcher: Searcher::new(config.tt_mb),
            difficulty: config.difficulty,
            rng: Rng::new(config.seed),
        }
    }

    /// 设置难度档位。
    pub fn set_difficulty(&mut self, difficulty: Difficulty) {
        self.difficulty = difficulty;
    }

    /// 当前难度档位。
    pub fn difficulty(&self) -> Difficulty {
        self.difficulty
    }

    /// 换局时调用：清空置换表与历史启发，避免上一局的数据干扰。
    ///
    /// **不清空会有一个很隐蔽的问题**：上一局的深层结果会污染这一局，
    /// 表现为「同一局面在不同对局里被评估成不同分数」。
    pub fn new_game(&mut self) {
        self.searcher.reset();
    }

    /// 按当前难度搜索，并施加该档位的随机化策略。
    pub fn search(&mut self, pos: &mut Position, stop: &dyn StopSignal) -> SearchResult {
        let profile = self.difficulty.profile();
        let mut result = self.searcher.search(pos, &profile.limits, stop);

        // 停止信号打断时结果不完整，不做随机化 —— 此时应尽快给出着法
        if !result.stopped {
            result.best_move = profile.choose(&mut self.rng, &result.root_moves);
        }
        result
    }

    /// 用**显式限制**搜索，不做难度随机化。
    ///
    /// 供三种场景使用：引擎自对弈标定、走棋提示（要最优着法而非「像人」的着法）、
    /// 战法讲解（需要精确的评分）。
    pub fn search_with(
        &mut self,
        pos: &mut Position,
        limits: SearchLimits,
        stop: &dyn StopSignal,
    ) -> SearchResult {
        self.searcher.search(pos, &limits, stop)
    }

    /// 只取一个推荐着法。
    pub fn best_move(&mut self, pos: &mut Position, stop: &dyn StopSignal) -> Option<Move> {
        self.search(pos, stop).best_move
    }

    /// 置换表命中率（调试用）。
    pub fn hit_rate(&self) -> f64 {
        self.searcher.tt.hit_rate()
    }
}

impl core::fmt::Debug for Engine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Engine")
            .field("difficulty", &self.difficulty)
            .field("tt_entries", &self.searcher.tt.capacity())
            .finish()
    }
}
