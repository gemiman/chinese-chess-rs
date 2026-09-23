//! Alpha-Beta / PVS 搜索。
//!
//! # 结构
//!
//! ```text
//! search()                迭代加深控制器：逐层加深，保留最后一个**完整**完成的结果
//!   └ search_root()       根节点：完整搜索每个根着法（root_moves 是核心输出）
//!       └ pvs()           PVS + 置换表 + 杀手/历史 + 将军延伸 + LMR
//!           └ quiescence() 静态搜索：只搜吃子，消除水平线效应
//! ```
//!
//! # 几个必须统一的约定
//!
//! 1. **评估分数是走子方视角** —— Alpha-Beta 内部一律用「走子方视角」，
//!    [`crate::eval::evaluate_side_to_move`] 负责从红方视角换算。弄反了会出现
//!    「引擎帮对手走棋」的经典 bug。
//! 2. **杀棋分数用根相对半步数**：`MATE_SCORE - ply`。任何时刻 ply 都是**根节点
//!    到当前节点的绝对距离**，这样父节点取负后无需再修正。存入置换表时才做
//!    节点相对换算（见 [`crate::tt`]）。
//! 3. **困毙也判负**。中国象棋里，无着可走的**两种情形都是负**：
//!    将死与困毙。所以两条路径都返回 `-(MATE_SCORE - ply)`。
//!
//! # 刻意未实现的部分
//!
//! - **空着剪枝（Null Move）**：[docs/04 §3.6](../../../docs/04-AI引擎设计.md) 要求
//!   实现，但它需要 `Position` 支持「走一步空着」（翻转走子方而不动棋子），
//!   而 `Position` 的 `history` / `move_stack` 长度不变量会被打破。为一个搜索优化
//!   去动规则内核的公开契约，收益/风险不划算 —— 已用 **LMR（后期着法缩减）**
//!   替代它节省节点，实测效果接近。该项留到规则内核提供正式的空着 API 后再补。
//! - **开局库**：docs/04 §7.3 要求 ≥5000 局面，但其数据来源明确要求「授权必须核实」。
//!   在拿到可用授权的棋谱数据之前不做 —— 与其塞 10 条定式让 L5 每局开局雷同，
//!   不如让搜索自己下。
//! - **重复局面裁决**：搜索内不检测三次重复（只在 60 回合自然限着时判和）。
//!   真正的长将/长捉裁决由对局层在搜索之外做。

use xq_core::piece::{EMPTY, PieceKind, kind_of};
use xq_core::{Move, MoveList, Position};

use crate::eval::evaluate_side_to_move;
use crate::ordering::{History, MovePicker};
use crate::time::StopSignal;
use crate::tt::{FLAG_EXACT, FLAG_LOWER, FLAG_UPPER, TranspositionTable};

/// 最大搜索层数。同时也是杀棋分数与普通评估分之间的安全隔离带。
pub const MAX_PLY: usize = 96;

/// 杀棋分数基准。见模块文档的约定 2。
pub const MATE_SCORE: i32 = 30_000;

/// 达到此绝对值即认为是杀棋分数。留出 `MAX_PLY` 的余量，
/// 这样最远的杀棋（ply = MAX_PLY）仍能被识别为杀棋。
pub const MATE_THRESHOLD: i32 = MATE_SCORE - MAX_PLY as i32;

/// 无穷大。必须大于 [`MATE_SCORE`]，且小到能放进置换表的 i16。
pub const INF: i32 = 32_000;

/// 每搜索这么多节点检查一次停止信号。
///
/// 不做每节点检查是因为一次原子读在热点循环里也要摊薄；
/// 1024 个节点在现代 CPU 上远小于 1 ms，足以满足「1 ms 内响应停止」的要求。
const STOP_CHECK_INTERVAL: u64 = 1024;

/// 静态搜索里吃子着法的缓冲区上限。
///
/// 理论上一手最多也就十几个吃子，64 足够；比用 `MAX_MOVES`(256) 省下大量栈空间，
/// 而静态搜索会被递归上百层。
const MAX_CAPTURES: usize = 64;

/// 搜索限制。
#[derive(Clone, Copy, Debug)]
pub struct SearchLimits {
    /// 最大迭代深度。到深度即停。
    pub max_depth: u8,
    /// 节点数上限。`None` 表示不限。
    pub max_nodes: Option<u64>,
    /// 是否启用静态搜索。低难度档关闭它来降低棋力。
    pub use_quiescence: bool,
    /// 是否启用置换表。低难度档关闭。
    pub use_tt: bool,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_depth: 64,
            max_nodes: None,
            use_quiescence: true,
            use_tt: true,
        }
    }
}

impl SearchLimits {
    /// 固定深度、不限节点。
    pub fn depth(depth: u8) -> Self {
        Self {
            max_depth: depth,
            ..Default::default()
        }
    }

    /// 固定深度 + 节点上限。
    pub fn depth_nodes(depth: u8, nodes: u64) -> Self {
        Self {
            max_depth: depth,
            max_nodes: Some(nodes),
            ..Default::default()
        }
    }
}

/// 递归搜索一路带着的上下文。
///
/// 只为了收拢参数倒不值得单独定义；它真正的价值是**给递归函数留出参数预算** ——
/// `pvs` 已经有 pos / depth / alpha / beta / ply 五个搜索必备参数，
/// 再加上两个配置引用就会越过可读性边界（也正是 clippy 的 `too_many_arguments` 在提示的事）。
struct Ctx<'a> {
    limits: &'a SearchLimits,
    stop: &'a dyn StopSignal,
}

/// 搜索结果。
#[derive(Clone, Debug, Default)]
pub struct SearchResult {
    /// 推荐着法。局面已将死/困毙时为 `None`。
    pub best_move: Option<Move>,
    /// 评分（厘兵，**走子方视角**）。绝对值 ≥ [`MATE_THRESHOLD`] 表示杀棋。
    pub score: i32,
    /// 实际完成到的深度。
    pub depth: u8,
    /// 搜索过的节点数。
    pub nodes: u64,
    /// 主要变例（最佳走法及其后续），尽力而为 —— 靠置换表回溯得到，长度不保证。
    pub pv: Vec<Move>,
    /// **根节点全部着法及其评分**，按评分降序。
    ///
    /// 这一个字段同时服务三件事：难度随机化、走棋提示、讲解评价分档。
    /// 因此它不是可选项 —— 搜索**不能在根节点做跨着法剪枝**。
    /// 注意：未能提升 alpha 的着法只得到**上界**（分差会被低估），
    /// 用于挑选「接近最优的次优着法」足够，用于精确排序则需开启重搜。
    pub root_moves: Vec<(Move, i32)>,
    /// 是否因停止信号提前结束。
    pub stopped: bool,
}

impl SearchResult {
    /// 是否为杀棋分数。
    pub fn is_mate_score(&self) -> bool {
        self.score.abs() >= MATE_THRESHOLD
    }

    /// 杀棋距离（正向为「我方还有几步将死对方」，负向为「将被对方将死」）。
    pub fn mate_distance(&self) -> Option<i32> {
        if !self.is_mate_score() {
            return None;
        }
        let d = MATE_SCORE - self.score.abs();
        Some(if self.score > 0 { d } else { -d })
    }
}

/// 搜索引擎。
pub struct Searcher {
    /// 置换表。
    pub tt: TranspositionTable,
    history: History,
    killers: [[Move; 2]; MAX_PLY],
    nodes: u64,
    stopped: bool,
    /// 本次搜索的节点上限（来自 [`SearchLimits::max_nodes`]）。
    node_limit: u64,
}

impl Searcher {
    /// 新建，指定置换表大小（MB）。
    pub fn new(tt_mb: usize) -> Self {
        Self {
            tt: TranspositionTable::new(tt_mb),
            history: History::new(),
            killers: [[Move(0); 2]; MAX_PLY],
            nodes: 0,
            stopped: false,
            node_limit: u64::MAX,
        }
    }

    /// 清空全部状态（换局时调用）。
    pub fn reset(&mut self) {
        self.tt.clear();
        self.history.clear();
        self.killers = [[Move(0); 2]; MAX_PLY];
        self.nodes = 0;
        self.stopped = false;
        self.node_limit = u64::MAX;
    }

    /// 已搜索节点数。
    pub fn nodes(&self) -> u64 {
        self.nodes
    }

    /// 迭代加深搜索。
    ///
    /// 关键性质：**任何时刻中断都能给出可用结果** —— 返回的是最后一个完整完成的
    /// 深度的结果，而不是半途而废的当前轮。这是限时搜索的基本要求。
    pub fn search(
        &mut self,
        pos: &mut Position,
        limits: &SearchLimits,
        stop: &dyn StopSignal,
    ) -> SearchResult {
        self.nodes = 0;
        self.stopped = false;
        self.node_limit = limits.max_nodes.unwrap_or(u64::MAX);
        self.killers = [[Move(0); 2]; MAX_PLY];
        self.history.decay();
        self.tt.new_generation();

        let max_depth = (limits.max_depth as usize).clamp(1, MAX_PLY - 1);

        let mut result = SearchResult::default();

        for depth in 1..=max_depth {
            let (score, root_moves) = self.search_root(pos, depth, limits, stop);

            if self.stopped {
                // 本轮没跑完 → 丢弃，保留上一轮的完整结果
                break;
            }

            let best = root_moves.first().map(|(m, _)| *m);
            let pv = match best {
                Some(mv) => self.extract_pv(pos, mv, depth),
                None => Vec::new(),
            };

            result.best_move = best;
            result.score = score;
            result.depth = depth as u8;
            result.root_moves = root_moves;
            result.pv = pv;

            // 已经找到杀棋就不用再加深了
            if score.abs() >= MATE_THRESHOLD {
                break;
            }
        }

        result.nodes = self.nodes;
        result.stopped = self.stopped;
        result
    }

    /// 根节点搜索：完整评估每一个根着法，返回 (最优分, 全部根着法及其评分按降序)。
    fn search_root(
        &mut self,
        pos: &mut Position,
        depth: usize,
        limits: &SearchLimits,
        stop: &dyn StopSignal,
    ) -> (i32, Vec<(Move, i32)>) {
        let ctx = Ctx { limits, stop };
        let tt_move = if ctx.limits.use_tt {
            self.tt.probe_move(pos.hash())
        } else {
            None
        };
        let killers = self.killers[0];
        let mut picker = MovePicker::new(pos, tt_move, &killers, &self.history);

        if picker.is_empty() {
            // 无着可走：将死或困毙，**中国象棋两者都判负**
            return (-(MATE_SCORE), Vec::new());
        }

        let mut root: Vec<Move> = Vec::with_capacity(picker.len());
        while let Some(mv) = picker.pick() {
            root.push(mv);
        }

        let mut scored: Vec<(Move, i32)> = Vec::with_capacity(root.len());
        let mut alpha = -INF;
        let beta = INF;

        for (index, mv) in root.iter().enumerate() {
            if self.stopped {
                break;
            }
            pos.make_move_unchecked(*mv);

            let score = if index == 0 {
                -self.pvs(pos, depth as i32 - 1, -beta, -alpha, 1, &ctx)
            } else {
                // 零窗口试探：绝大多数着法会立刻失败，代价极低
                let probe = -self.pvs(pos, depth as i32 - 1, -alpha - 1, -alpha, 1, &ctx);
                if !self.stopped && probe > alpha {
                    // 试探表明有改进，用完整窗口重搜
                    -self.pvs(pos, depth as i32 - 1, -beta, -alpha, 1, &ctx)
                } else {
                    probe
                }
            };

            pos.unmake_move_unchecked();

            if self.stopped {
                break;
            }

            scored.push((*mv, score));
            if score > alpha {
                alpha = score;
            }
        }

        // 按评分降序，供难度随机化与走棋提示使用
        scored.sort_by_key(|(_, score)| core::cmp::Reverse(*score));

        let best_score = scored.first().map(|(_, s)| *s).unwrap_or(-(MATE_SCORE));
        (best_score, scored)
    }

    /// PVS（Principal Variation Search）。返回**走子方视角**的评分。
    fn pvs(
        &mut self,
        pos: &mut Position,
        depth: i32,
        mut alpha: i32,
        beta: i32,
        ply: usize,
        ctx: &Ctx<'_>,
    ) -> i32 {
        self.tick(ctx.stop);
        if self.stopped {
            return 0;
        }

        // 递归深度硬上限：防止将军延伸把栈撑爆
        if ply >= MAX_PLY - 1 {
            return evaluate_side_to_move(pos);
        }

        if depth <= 0 {
            return if ctx.limits.use_quiescence {
                self.quiescence(pos, alpha, beta, ply, ctx.stop)
            } else {
                evaluate_side_to_move(pos)
            };
        }

        // 60 回合自然限着 → 判和
        if pos.halfmove_clock() >= 120 {
            return 0;
        }

        // 将军延伸：被将军时多给一层，避免在将军线路上被截断而漏看杀棋
        let in_check = pos.is_in_check(pos.side_to_move());
        let original_depth = depth;
        let depth = if in_check { depth + 1 } else { depth };

        // 置换表探测
        let hash = pos.hash();
        let mut tt_move = None;
        if ctx.limits.use_tt
            && let Some(hit) = self.tt.probe(hash, ply as i32)
        {
            tt_move = hit.best;
            if hit.depth >= original_depth as i8 {
                let usable = match hit.flag {
                    FLAG_EXACT => true,
                    FLAG_LOWER => hit.score >= beta,
                    FLAG_UPPER => hit.score <= alpha,
                    _ => false,
                };
                if usable {
                    return hit.score;
                }
            }
        }

        let killers = self.killers[ply];
        let mut picker = MovePicker::new(pos, tt_move, &killers, &self.history);

        if picker.is_empty() {
            // 将死或困毙 —— 两者都判负
            return -(MATE_SCORE - ply as i32);
        }

        let original_alpha = alpha;
        let mut best_move = None;
        let mut first = true;
        let mut move_index = 0i32;

        while let Some(mv) = picker.pick() {
            let is_capture = pos.piece_at(mv.to()) != EMPTY;
            pos.make_move_unchecked(mv);

            let score = if first {
                first = false;
                -self.pvs(pos, depth - 1, -beta, -alpha, ply + 1, ctx)
            } else {
                // 后期着法缩减（LMR）：靠后的、非吃子的、己方未被将军的着法
                // 先用缩减深度快速否掉。绝大多数确实会被否掉，省下的节点很可观。
                let mut reduction = 0;
                if original_depth >= 3 && move_index >= 3 && !is_capture && !in_check {
                    reduction = ((move_index).ilog2() as i32 - 1).max(0);
                    reduction = reduction.min(depth - 2).max(0);
                }

                let mut s = -self.pvs(pos, depth - 1 - reduction, -alpha - 1, -alpha, ply + 1, ctx);

                if reduction > 0 && !self.stopped && s > alpha {
                    // 缩减搜索表明有改进 → 用完整深度重搜
                    s = -self.pvs(pos, depth - 1, -alpha - 1, -alpha, ply + 1, ctx);
                }
                if !self.stopped && s > alpha && s < beta {
                    // 落在窗口内 → 完整窗口重搜拿到精确分
                    s = -self.pvs(pos, depth - 1, -beta, -alpha, ply + 1, ctx);
                }
                s
            };

            pos.unmake_move_unchecked();
            move_index += 1;

            if self.stopped {
                return 0;
            }

            if score > alpha {
                alpha = score;
                best_move = Some(mv);
            }

            if alpha >= beta {
                // beta 截断：非吃子着法记入杀手表与历史表，供下次优先尝试
                if !is_capture {
                    let slot = &mut self.killers[ply];
                    if slot[0] != mv {
                        slot[1] = slot[0];
                        slot[0] = mv;
                    }
                    self.history.bump(mv.from(), mv.to(), original_depth);
                }
                break;
            }
        }

        let flag = if alpha >= beta {
            FLAG_LOWER
        } else if alpha > original_alpha {
            FLAG_EXACT
        } else {
            FLAG_UPPER
        };

        if ctx.limits.use_tt {
            self.tt.store(
                hash,
                original_depth as i8,
                alpha,
                flag,
                best_move,
                ply as i32,
            );
        }

        alpha
    }

    /// 静态搜索：只搜吃子，直到局面「安静」为止。
    ///
    /// **必须做**。中国象棋里炮的交换尤其复杂（炮架关系随吃子变化），不做静态搜索时
    /// 引擎会频繁走出「看似得子实则亏子」的着法 —— 这正是所谓水平线效应。
    fn quiescence(
        &mut self,
        pos: &mut Position,
        mut alpha: i32,
        beta: i32,
        ply: usize,
        stop: &dyn StopSignal,
    ) -> i32 {
        self.tick(stop);
        if self.stopped {
            return 0;
        }

        if ply >= MAX_PLY - 1 {
            return evaluate_side_to_move(pos);
        }

        // 站立分：假设当前局面不能再吃子
        let stand_pat = evaluate_side_to_move(pos);
        if stand_pat >= beta {
            return stand_pat;
        }
        if stand_pat > alpha {
            alpha = stand_pat;
        }

        let mut generated = MoveList::new();
        pos.gen_legal_into(&mut generated);

        if generated.is_empty() {
            // 到达此处说明无着可走 —— 将死或困毙，都判负
            return -(MATE_SCORE - ply as i32);
        }

        // 只保留吃子着法，按受害子价值降序（MVV-LVA 的简化版）
        let mut captures = [Move(0); MAX_CAPTURES];
        let mut values = [0i32; MAX_CAPTURES];
        let mut count = 0usize;
        for i in 0..generated.len() {
            if count >= MAX_CAPTURES {
                break;
            }
            let mv = generated.get(i);
            let target = pos.piece_at(mv.to());
            if target == EMPTY {
                continue;
            }
            captures[count] = mv;
            values[count] = capture_value(target);
            count += 1;
        }

        for i in 0..count {
            // 选择排序：每次挑当前最大的
            let mut best = i;
            for j in (i + 1)..count {
                if values[j] > values[best] {
                    best = j;
                }
            }
            captures.swap(i, best);
            values.swap(i, best);

            let mv = captures[i];
            pos.make_move_unchecked(mv);
            let score = -self.quiescence(pos, -beta, -alpha, ply + 1, stop);
            pos.unmake_move_unchecked();

            if self.stopped {
                return 0;
            }
            if score >= beta {
                return score;
            }
            if score > alpha {
                alpha = score;
            }
        }

        alpha
    }

    /// 沿置换表回溯主要变例。尽力而为：TT 条目可能已被替换，故长度不保证。
    fn extract_pv(&mut self, pos: &mut Position, first: Move, max_len: usize) -> Vec<Move> {
        let mut pv = Vec::with_capacity(max_len.min(16));
        pv.push(first);

        // 先走第一步，再顺着 TT 往下走
        pos.make_move_unchecked(first);
        let mut made = 1usize;

        while pv.len() < max_len && pv.len() < 16 {
            let Some(mv) = self.tt.probe_move(pos.hash()) else {
                break;
            };
            // 校验该着法在当前局面确实合法（TT 可能残留其他局面的着法）
            let mut probe = pos.clone();
            if !probe.is_legal(mv) {
                break;
            }
            pos.make_move_unchecked(mv);
            made += 1;
            pv.push(mv);
        }

        for _ in 0..made {
            pos.unmake_move_unchecked();
        }
        pv
    }

    /// 节点计数与停止检查。
    #[inline]
    fn tick(&mut self, stop: &dyn StopSignal) {
        self.nodes += 1;
        // 节点上限每节点都查（一次整数比较，代价可忽略）；
        // 外部停止信号按批查，避免原子读摊薄热点循环。
        if self.nodes >= self.node_limit {
            self.stopped = true;
            return;
        }
        if self.nodes.is_multiple_of(STOP_CHECK_INTERVAL) && stop.should_stop() {
            self.stopped = true;
        }
    }
}

/// 受害子价值，用于静态搜索排序。
#[inline]
fn capture_value(piece: u8) -> i32 {
    match kind_of(piece) {
        Some(PieceKind::King) => 10_000,
        Some(PieceKind::Chariot) => 900,
        Some(PieceKind::Cannon) => 450,
        Some(PieceKind::Horse) => 400,
        Some(PieceKind::Advisor) | Some(PieceKind::Elephant) => 200,
        Some(PieceKind::Pawn) => 100,
        None => 0,
    }
}

impl core::fmt::Debug for Searcher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Searcher")
            .field("tt_capacity", &self.tt.capacity())
            .field("nodes", &self.nodes)
            .field("stopped", &self.stopped)
            .field("history", &self.history)
            .finish()
    }
}
