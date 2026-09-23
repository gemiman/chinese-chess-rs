//! 战术识别：L1 结构层 / L2 关系层 / L3 棋形层。
//!
//! # 分层依据：可判定性
//!
//! | 层 | 判定方式 | 可靠性 |
//! |---|---|---|
//! | L1 结构层 | 由 `xq-core` 的局面语义**精确判定** | 100% 确定 |
//! | L2 关系层 | 攻击关系 + 交换价值计算 | 高（可测试） |
//! | L3 棋形层 | **声明式棋形库匹配**（外部 JSON） | 取决于棋形定义质量 |
//!
//! # 攻击关系怎么查：复用 xq-core 而不是另写一套
//!
//! 需要「谁在攻击这一格」而不只是「这一格是否被攻击」。做法是**逐个摘除候选子**：
//! 摘掉某子后该格不再被攻击，则它就是攻击者。这样攻击语义完全由
//! [`Position::is_attacked`] 决定 —— 不会出现「讲解说在将军、而规则说不算」这种
//! 两套实现打架的问题（那是本项目最不能接受的错误类型）。
//!
//! 代价是每次查询要重建一个局面（90 格扫描 + 哈希），约 1~2 µs。
//! 对「≤ 5 ms」的本地延迟目标绰绰有余。

use xq_core::piece::{EMPTY, PieceKind, color_of, kind_of};
use xq_core::square::{BOARD_SIZE, col_of, index, row_of};
use xq_core::{Color, Move, MoveNature, Position};

use crate::knowledge::{ConstraintDef, KnowledgeBase};
use crate::note::{TacticCategory, TacticTag};

/// 一次识别的完备输入。
#[derive(Clone, Copy, Debug)]
pub struct MoveContext {
    /// 走子方。
    pub side: Color,
    /// 走的那一步。
    pub mv: Move,
    /// `xq-core` 给出的着法性质。
    pub nature: MoveNature,
    /// 走子前是否被将军。
    pub was_in_check: bool,
    /// 被吃掉的棋子（`EMPTY` 表示未吃子）。
    pub captured: u8,
    /// 走子前的评分（走子方视角，来自引擎 `root_moves` 的最优分）。
    pub score_best: i32,
    /// 实际着法的分差（厘兵）。
    pub score_loss: i32,
}

/// 战术识别器。
#[derive(Debug)]
pub struct TacticDetector<'a> {
    kb: &'a KnowledgeBase,
}

/// 摘掉某一格上的棋子，得到一个副本。
///
/// 将帅不参与摘除（摘掉将帅会让局面非法）。
fn without(pos: &Position, sq: u8) -> Option<Position> {
    let piece = pos.piece_at(sq);
    if piece == EMPTY || kind_of(piece) == Some(PieceKind::King) {
        return None;
    }
    let mut squares = *pos.squares();
    squares[sq as usize] = EMPTY;
    Position::from_squares(
        squares,
        pos.side_to_move(),
        pos.halfmove_clock(),
        pos.fullmove_number(),
    )
    .ok()
}

/// `from` 处的棋子是否攻击 `to`。
fn piece_attacks(pos: &Position, from: u8, to: u8, color: Color) -> bool {
    if !pos.is_attacked(to, color) {
        return false;
    }
    match without(pos, from) {
        // 摘掉它后不再被攻击 → 攻击者就是它
        Some(reduced) => !reduced.is_attacked(to, color),
        // 将帅不摘除：退化为「是否被攻击」的乐观判断（仅影响将帅贴脸这类极少数情形）
        None => color_of(pos.piece_at(from)) == Some(color),
    }
}

/// 枚举攻击 `target` 的 `by` 方棋子。
///
/// 将帅不列入（它只在贴脸与白脸将时构成攻击，棋形谓词不依赖这一点）。
fn attackers_of(pos: &Position, target: u8, by: Color) -> Vec<u8> {
    let mut out = Vec::new();
    if !pos.is_attacked(target, by) {
        return out;
    }
    for sq in 0..BOARD_SIZE as u8 {
        let piece = pos.piece_at(sq);
        if color_of(piece) != Some(by) || kind_of(piece) == Some(PieceKind::King) {
            continue;
        }
        if let Some(reduced) = without(pos, sq)
            && !reduced.is_attacked(target, by)
        {
            out.push(sq);
        }
    }
    out
}

/// 排序用的子力价值（与 `xq-ai` 的排序表同尺）。
const fn piece_value(kind: PieceKind) -> i32 {
    match kind {
        PieceKind::King => 10_000,
        PieceKind::Chariot => 900,
        PieceKind::Cannon => 450,
        PieceKind::Horse => 400,
        PieceKind::Advisor | PieceKind::Elephant => 200,
        PieceKind::Pawn => 100,
    }
}

/// 该格是否被 `color` 方保护（即 `color` 的某个棋子攻击它）。
fn is_defended(pos: &Position, sq: u8, color: Color) -> bool {
    !attackers_of(pos, sq, color).is_empty()
}

impl<'a> TacticDetector<'a> {
    /// 新建。
    pub fn new(kb: &'a KnowledgeBase) -> Self {
        Self { kb }
    }

    /// 识别一步棋的全部战术。
    ///
    /// `pos_before` 是走子**前**的局面，`pos_after` 是走子**后**的（此时轮到对方）。
    /// 返回的标签按置信度降序。
    pub fn detect(
        &self,
        pos_before: &Position,
        pos_after: &Position,
        ctx: &MoveContext,
    ) -> Vec<TacticTag> {
        let mut hits: Vec<(String, f32)> = Vec::new();

        self.detect_l1(pos_after, ctx, &mut hits);
        self.detect_l2(pos_before, pos_after, ctx, &mut hits);
        self.detect_l3(pos_after, ctx, &mut hits);
        self.detect_opening_move(pos_after, ctx, &mut hits);

        // 附上知识库里的名称与类别
        let mut tags: Vec<TacticTag> = hits
            .into_iter()
            .filter_map(|(id, confidence)| {
                let def = self.kb.tactic(&id)?;
                Some(TacticTag {
                    name: def.name.clone(),
                    category: TacticCategory::parse(&def.category)
                        .unwrap_or(TacticCategory::Relation),
                    id,
                    confidence,
                })
            })
            .collect();

        tags.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        tags.dedup_by(|a, b| a.id == b.id);
        tags
    }

    /// 用知识库里的 `confidence_base` 记录一次命中。
    fn push(&self, id: &str, scale: f32, hits: &mut Vec<(String, f32)>) {
        let Some(def) = self.kb.tactic(id) else {
            // 素材里没有这条战术 —— 静默跳过（不 panic：讲解是增强功能）
            debug_assert!(false, "知识库里缺少战术 {id}");
            return;
        };
        let confidence = (def.confidence_base * scale).clamp(0.0, 1.0);
        hits.push((id.to_string(), confidence));
    }

    // ------------------------------------------------------------ 开局着法

    /// 识别「**本步本身就是某个开局的名称来源**」。
    ///
    /// 为什么不走开局序列匹配：序列匹配要等定式走完（甚至好几步）才敢下结论，
    /// 而用户走了第一步就想知道「我下的是什么体系」。这类判定只看本步的几何特征，
    /// 不依赖对局历史。
    ///
    /// 反宫马、单提马、顺炮、列炮这类**需要同时知道双方着法**的定式不在这里判 ——
    /// 它们由 [`crate::opening::match_opening`] 的序列匹配负责。
    fn detect_opening_move(
        &self,
        pos_after: &Position,
        ctx: &MoveContext,
        hits: &mut Vec<(String, f32)>,
    ) {
        let Some(kind) = kind_of(pos_after.piece_at(ctx.mv.to())) else {
            return;
        };
        let from_col = col_of(ctx.mv.from());
        let to_col = col_of(ctx.mv.to());

        match kind {
            // 中炮：炮从 b/h 路平到中路 —— 最主流的开局体系
            PieceKind::Cannon if matches!(from_col, 1 | 7) && to_col == 4 => {
                self.push("opening_central_cannon", 1.0, hits);
            }
            // 过宫炮：炮横向越过中路到另一侧（起止分居中路两侧，且不落在中路）
            PieceKind::Cannon
                if to_col != 4 && from_col != to_col && (from_col < 4) != (to_col < 4) =>
            {
                self.push("opening_cross_palace_cannon", 0.85, hits);
            }
            // 士角炮：炮平到仕角所在的 d / f 路
            PieceKind::Cannon if matches!(to_col, 3 | 5) => {
                self.push("opening_advisor_corner_cannon", 0.8, hits);
            }
            // 仙人指路：七路或三路的兵卒向前一步（不吃子）
            PieceKind::Pawn if matches!(from_col, 2 | 6) && ctx.captured == EMPTY => {
                self.push("opening_pawn_probe", 0.9, hits);
            }
            // 飞相局：相 / 象出动
            PieceKind::Elephant => {
                self.push("opening_elephant", 0.8, hits);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ L1

    fn detect_l1(&self, pos_after: &Position, ctx: &MoveContext, hits: &mut Vec<(String, f32)>) {
        let enemy = ctx.side.opponent();
        let enemy_king = pos_after.king_index(enemy);
        let in_check = pos_after.is_in_check(enemy);

        // 将死 / 困毙需要终局状态 —— 两者都判负
        let status = {
            let mut probe = pos_after.clone();
            probe.status()
        };
        match status {
            xq_core::GameStatus::Checkmate { .. } => self.push("mate", 1.0, hits),
            xq_core::GameStatus::Stalemate { .. } => self.push("stalemate_win", 1.0, hits),
            _ => {}
        }

        if in_check {
            self.push("check", 1.0, hits);

            // 双将：攻击者 ≥ 2
            let attackers = attackers_of(pos_after, enemy_king, ctx.side);
            if attackers.len() >= 2 {
                self.push("double_check", 1.0, hits);
            }

            // 闪将：将军者不是本步走的那枚棋子
            if !attackers.is_empty() && !attackers.contains(&ctx.mv.to()) {
                self.push("discovered_check", 0.95, hits);
            }
        }

        if ctx.captured != EMPTY {
            self.push("capture", 1.0, hits);
        }

        // 解将与它的三种方式直接复用 xq-core 已判定的着法性质
        if ctx.was_in_check {
            self.push("resolve_check", 1.0, hits);
            match ctx.nature {
                MoveNature::Interpose => self.push("interpose", 1.0, hits),
                MoveNature::Escape => self.push("king_escape", 1.0, hits),
                MoveNature::CaptureAttacker => self.push("capture_attacker", 1.0, hits),
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------ L2

    fn detect_l2(
        &self,
        pos_before: &Position,
        pos_after: &Position,
        ctx: &MoveContext,
        hits: &mut Vec<(String, f32)>,
    ) {
        let us = ctx.side;
        let enemy = us.opponent();
        let attacker = ctx.mv.to();
        let attacker_kind = kind_of(pos_after.piece_at(attacker));

        // ---- 捉双 / 威胁：本步棋子攻击到的敌方棋子（不含将帅） ----
        let mut profitable_victims: Vec<u8> = Vec::new();
        let mut all_victims: Vec<u8> = Vec::new();
        for sq in 0..BOARD_SIZE as u8 {
            let piece = pos_after.piece_at(sq);
            if piece == EMPTY || color_of(piece) != Some(enemy) {
                continue;
            }
            if kind_of(piece) == Some(PieceKind::King) {
                continue;
            }
            if !piece_attacks(pos_after, attacker, sq, us) {
                continue;
            }
            all_victims.push(sq);

            // 简化交换评估（一层）：吃它之后对方能否吃回来，且收益为正
            let victim_value = kind_of(piece).map(piece_value).unwrap_or(0);
            let recapture_cost = if is_defended(pos_after, sq, enemy) {
                attacker_kind.map(piece_value).unwrap_or(0)
            } else {
                0
            };
            if victim_value - recapture_cost > 0 {
                profitable_victims.push(sq);
            }
        }

        if profitable_victims.len() >= 2 {
            // 置信度：损失最大的两个目标之和 / 400
            let mut values: Vec<i32> = profitable_victims
                .iter()
                .filter_map(|sq| kind_of(pos_after.piece_at(*sq)).map(piece_value))
                .collect();
            values.sort_unstable_by(|a, b| b.cmp(a));
            let top_two: i32 = values.iter().take(2).sum();
            let scale = (top_two as f32 / 400.0).clamp(0.3, 1.0);
            self.push("fork", scale, hits);
        } else if !profitable_victims.is_empty() {
            self.push("threat", 0.85, hits);
        } else if !all_victims.is_empty() {
            self.push("threat", 0.6, hits);
        }

        // ---- 牵制 / 串打 ----
        //
        // ⚠️ **只对车判定**。炮的「牵制」要复杂得多：炮吃子依赖炮架，前方棋子移开后
        // 炮架可能同时消失，于是「被牵制」这个结论根本不成立。实测中初版对炮也判牵制，
        // 结果第一步「炮二平五」就报出「牵制 e6 卒」—— 而那个卒其实随时可以走。
        // docs/05 §3.3 明确要求**宁可保守**：少判一个战术只是少一条讲解，
        // 误判则直接损害整条讲解的可信度。炮的牵制需要炮架不变性分析，留待后续。
        if attacker_kind == Some(PieceKind::Chariot) {
            let attacker_value = piece_value(PieceKind::Chariot);
            for sq in 0..BOARD_SIZE as u8 {
                let target = pos_after.piece_at(sq);
                if target == EMPTY || color_of(target) != Some(enemy) {
                    continue;
                }
                let target_kind = match kind_of(target) {
                    Some(k) => k,
                    None => continue,
                };
                if !piece_attacks(pos_after, attacker, sq, us) {
                    continue;
                }
                let Some(behind) = piece_behind_on_line(pos_after, attacker, sq, enemy) else {
                    continue;
                };
                let behind_kind = match kind_of(pos_after.piece_at(behind)) {
                    Some(k) => k,
                    None => continue,
                };
                let behind_value = piece_value(behind_kind);
                let target_value = piece_value(target_kind);

                // 牵制：后方目标更值钱（动前方的子会暴露它）
                if behind_value > target_value && behind_value >= attacker_value {
                    self.push("pin", 0.8, hits);
                }
                // 串打：前方目标更值钱（逼它移动后吃掉后方）
                if target_value > behind_value && target_value >= attacker_value {
                    self.push("skewer", 0.75, hits);
                }
            }
        }

        // ---- 闪击 ----
        //
        // 严格定义：本步棋子**原本挡在**某条线上，移开之后才让另一枚棋子的攻击成立。
        // 只比较「走子前后是否被攻击」是不够的 —— 那样把任何新出现的攻击都算成闪击，
        // 实测中第一步就误报了一次。这里额外要求：走子前的位置必须落在那条线上、
        // 且位于攻击者与目标之间。
        let from = ctx.mv.from();
        {
            let mut discovered = false;
            'outer: for sq in 0..BOARD_SIZE as u8 {
                let piece = pos_after.piece_at(sq);
                if piece == EMPTY || color_of(piece) != Some(enemy) {
                    continue;
                }
                if kind_of(piece) == Some(PieceKind::King) {
                    continue;
                }
                for origin in 0..BOARD_SIZE as u8 {
                    if origin == attacker {
                        continue;
                    }
                    if color_of(pos_after.piece_at(origin)) != Some(us) {
                        continue;
                    }
                    if !piece_attacks(pos_after, origin, sq, us) {
                        continue;
                    }
                    // 走子前没有这条攻击，且原来的位置正好挡在中间 → 闪击
                    if !piece_attacks(pos_before, origin, sq, us) && lies_between(origin, sq, from)
                    {
                        discovered = true;
                        break 'outer;
                    }
                }
            }
            if discovered {
                self.push("discovered_attack", 0.82, hits);
            }
        }

        // ---- 邀兑 / 弃子：本步棋子落点上的得失 ----
        let attacker_attacked = is_defended(pos_after, attacker, enemy);
        let attacker_defended = is_defended(pos_after, attacker, us);
        if attacker_attacked && !attacker_defended {
            if let Some(kind) = attacker_kind {
                // 被等价子或更小的子攻击且无保护 → 弃子
                let cheapest_attacker = attackers_of(pos_after, attacker, enemy)
                    .iter()
                    .filter_map(|sq| kind_of(pos_after.piece_at(*sq)).map(piece_value))
                    .min()
                    .unwrap_or(10_000);
                if cheapest_attacker <= piece_value(kind) {
                    self.push("sacrifice", 0.7, hits);
                } else {
                    self.push("exchange_offer", 0.65, hits);
                }
            }
        } else if attacker_attacked && attacker_defended && ctx.captured != EMPTY {
            self.push("exchange_offer", 0.6, hits);
        }

        // ---- 得子 / 失子 ----
        if ctx.captured != EMPTY {
            let victim_value = kind_of(ctx.captured).map(piece_value).unwrap_or(0);
            let attacker_value = attacker_kind.map(piece_value).unwrap_or(0);
            if victim_value - attacker_value >= 200 {
                self.push("win_material", 0.9, hits);
            }
        }
        if ctx.score_loss >= crate::assess::THRESHOLD_DUBIOUS {
            self.push("lose_material", 0.6, hits);
        }

        // ---- 通线：本步让己方的车或炮获得了一条畅通线路 ----
        if let Some(kind) = attacker_kind
            && matches!(kind, PieceKind::Chariot | PieceKind::Cannon)
        {
            let lines_before = open_lines(pos_before, ctx.mv.from(), us);
            let lines_after = open_lines(pos_after, attacker, us);
            if lines_after > lines_before {
                self.push("open_file", 0.7, hits);
            }
        }

        // ---- 中路控制 ----
        if attacker_kind.is_some() && col_of(attacker) == 4 {
            self.push("central_control", 0.7, hits);
        }
    }

    // ------------------------------------------------------------ L3

    fn detect_l3(&self, pos_after: &Position, ctx: &MoveContext, hits: &mut Vec<(String, f32)>) {
        let attacker_kind = match kind_of(pos_after.piece_at(ctx.mv.to())) {
            Some(k) => k,
            None => return,
        };

        for tactic in &self.kb.tactics {
            let Some(pattern) = &tactic.pattern else {
                continue;
            };
            // 攻击者种类要吻合（`any` 表示不限）
            let wanted = &pattern.attacker.kind;
            if wanted != "any" && !kind_matches(wanted, attacker_kind) {
                continue;
            }

            let pctx = PredicateCtx {
                pos_after,
                to: ctx.mv.to(),
                us: ctx.side,
                enemy: ctx.side.opponent(),
                captured: ctx.captured,
            };

            if pattern.constraints.iter().all(|c| eval_predicate(&pctx, c)) {
                // 棋形层置信度按素材基准值折扣 —— 素材仍是「初始定义，待校准」
                self.push(&tactic.id, 1.0, hits);
            }
        }
    }
}

/// 棋形里写的 kind 名是否与棋子种类一致。
fn kind_matches(name: &str, kind: PieceKind) -> bool {
    match name {
        "horse" => kind == PieceKind::Horse,
        "cannon" => kind == PieceKind::Cannon,
        "chariot" => kind == PieceKind::Chariot,
        "pawn" => kind == PieceKind::Pawn,
        "advisor" => kind == PieceKind::Advisor,
        "elephant" => kind == PieceKind::Elephant,
        "king" => kind == PieceKind::King,
        _ => true, // 未知识别名 → 不拦（宁可多判也不漏判棋形）
    }
}

/// `mid` 是否落在 `a` 与 `b` 构成的直线上、且位于两者之间（不含端点）。
fn lies_between(a: u8, b: u8, mid: u8) -> bool {
    let (ac, ar) = (col_of(a) as i8, row_of(a) as i8);
    let (bc, br) = (col_of(b) as i8, row_of(b) as i8);
    let (mc, mr) = (col_of(mid) as i8, row_of(mid) as i8);

    // 必须与两端共线
    let cross = (bc - ac) * (mr - ar) - (br - ar) * (mc - ac);
    if cross != 0 {
        return false;
    }
    // 必须在两端构成的包围盒内部
    mc > ac.min(bc) && mc < ac.max(bc) || mr > ar.min(br) && mr < ar.max(br)
}

/// 某条线上攻击者与目标之后的那枚棋子。
fn piece_behind_on_line(pos: &Position, attacker: u8, target: u8, by: Color) -> Option<u8> {
    let (ac, ar) = (col_of(attacker) as i8, row_of(attacker) as i8);
    let (tc, tr) = (col_of(target) as i8, row_of(target) as i8);

    let (dc, dr) = match (tc.cmp(&ac), tr.cmp(&ar)) {
        (core::cmp::Ordering::Equal, core::cmp::Ordering::Equal) => return None,
        (core::cmp::Ordering::Equal, o) => (
            0,
            if o == core::cmp::Ordering::Greater {
                1
            } else {
                -1
            },
        ),
        (o, core::cmp::Ordering::Equal) => (
            if o == core::cmp::Ordering::Greater {
                1
            } else {
                -1
            },
            0,
        ),
        // 非直线（马等）不构成牵制
        _ => return None,
    };

    let mut c = tc + dc;
    let mut r = tr + dr;
    while (0..9).contains(&c) && (0..10).contains(&r) {
        let sq = index(c as u8, r as u8);
        let piece = pos.piece_at(sq);
        if piece != EMPTY {
            return (color_of(piece) == Some(by)).then_some(sq);
        }
        c += dc;
        r += dr;
    }
    None
}

/// 某格上的车/炮在该方向上的畅通线路数（0~4）。
fn open_lines(pos: &Position, sq: u8, color: Color) -> u32 {
    let mut count = 0;
    for (dc, dr) in [(0i8, 1i8), (0, -1), (1, 0), (-1, 0)] {
        let mut c = col_of(sq) as i8 + dc;
        let mut r = row_of(sq) as i8 + dr;
        let mut length = 0;
        while (0..9).contains(&c) && (0..10).contains(&r) {
            let target = index(c as u8, r as u8);
            let piece = pos.piece_at(target);
            if piece != EMPTY {
                if color_of(piece) != Some(color) {
                    length += 1; // 能吃子也算「有作为」
                }
                break;
            }
            length += 1;
            c += dc;
            r += dr;
        }
        if length >= 3 {
            count += 1;
        }
    }
    count
}

// ---------------------------------------------------------------- 棋形谓词

/// 谓词求值上下文。
struct PredicateCtx<'p> {
    pos_after: &'p Position,
    to: u8,
    us: Color,
    enemy: Color,
    captured: u8,
}

impl PredicateCtx<'_> {
    fn enemy_king(&self) -> u8 {
        self.pos_after.king_index(self.enemy)
    }

    /// 攻击者所在的列。
    fn col(&self) -> u8 {
        col_of(self.to)
    }

    /// 攻击者所在的行。
    fn row(&self) -> u8 {
        row_of(self.to)
    }

    /// 攻击者与对方将帅是否在同一条直线上。
    fn on_same_line_as_king(&self) -> bool {
        let king = self.enemy_king();
        self.col() == col_of(king) || self.row() == row_of(king)
    }

    /// 对方将帅所在行列上、攻击者与将帅之间的棋子。
    fn pieces_between_attacker_and_king(&self) -> Vec<u8> {
        let king = self.enemy_king();
        let (ac, ar) = (self.col() as i8, self.row() as i8);
        let (kc, kr) = (col_of(king) as i8, row_of(king) as i8);
        let (dc, dr) = match (kc.cmp(&ac), kr.cmp(&ar)) {
            (core::cmp::Ordering::Equal, o) => (
                0,
                if o == core::cmp::Ordering::Greater {
                    1
                } else {
                    -1
                },
            ),
            (o, core::cmp::Ordering::Equal) => (
                if o == core::cmp::Ordering::Greater {
                    1
                } else {
                    -1
                },
                0,
            ),
            _ => return Vec::new(),
        };
        let mut out = Vec::new();
        let mut c = ac + dc;
        let mut r = ar + dr;
        while (0..9).contains(&c) && (0..10).contains(&r) {
            if (c, r) == (kc, kr) {
                break;
            }
            let sq = index(c as u8, r as u8);
            if self.pos_after.piece_at(sq) != EMPTY {
                out.push(sq);
            }
            c += dc;
            r += dr;
        }
        out
    }

    /// 对方将帅在九宫内的可走格。
    fn king_escape_squares(&self) -> Vec<u8> {
        let king = self.enemy_king();
        let (c, r) = (col_of(king) as i8, row_of(king) as i8);
        let mut out = Vec::new();
        for (dc, dr) in [(0i8, 1i8), (0, -1), (1, 0), (-1, 0)] {
            let (nc, nr) = (c + dc, r + dr);
            if (0..9).contains(&nc)
                && (0..10).contains(&nr)
                && xq_core::square::in_palace(self.enemy, nc as u8, nr as u8)
            {
                out.push(index(nc as u8, nr as u8));
            }
        }
        out
    }
}

/// 对方半场（对攻击者而言的「敌阵」）判定。
fn in_enemy_half(color: Color, row: u8) -> bool {
    xq_core::square::has_crossed_river(color, row)
}

/// 对某方格子的将帅底线行。
fn enemy_base_row(enemy: Color) -> u8 {
    match enemy {
        Color::Red => 0,
        Color::Black => 9,
    }
}

/// 切比雪夫距离。
fn chebyshev(a: u8, b: u8) -> u8 {
    let dc = (col_of(a) as i8 - col_of(b) as i8).unsigned_abs();
    let dr = (row_of(a) as i8 - row_of(b) as i8).unsigned_abs();
    dc.max(dr)
}

/// 对方九宫的四个角。
fn palace_corners(enemy: Color) -> [u8; 4] {
    let (lo, hi) = enemy.palace_rows();
    [index(3, lo), index(5, lo), index(3, hi), index(5, hi)]
}

/// 到九宫列区间 [3,5] 的水平距离。
fn horizontal_distance_to_palace(col: u8) -> u8 {
    col.saturating_sub(5).max(3u8.saturating_sub(col))
}

fn count_own_pieces(pos: &Position, color: Color, kind: PieceKind) -> u32 {
    let mut n = 0;
    for sq in 0..BOARD_SIZE as u8 {
        let piece = pos.piece_at(sq);
        if color_of(piece) == Some(color) && kind_of(piece) == Some(kind) {
            n += 1;
        }
    }
    n
}

fn parse_kind(name: &str) -> Option<PieceKind> {
    match name {
        "king" => Some(PieceKind::King),
        "advisor" => Some(PieceKind::Advisor),
        "elephant" => Some(PieceKind::Elephant),
        "horse" => Some(PieceKind::Horse),
        "chariot" => Some(PieceKind::Chariot),
        "cannon" => Some(PieceKind::Cannon),
        "pawn" => Some(PieceKind::Pawn),
        _ => None,
    }
}

/// 求值单条棋形约束。
///
/// 返回 `false` 即该棋形不成立。**所有谓词都按「宁可保守」实现** ——
/// 少判一个棋形只是少一条讲解，误判则会直接损害讲解可信度。
fn eval_predicate(ctx: &PredicateCtx<'_>, con: &ConstraintDef) -> bool {
    let pos = ctx.pos_after;
    match con.predicate.as_str() {
        // ---- 攻击关系 ----
        "attacker_checks_king" => {
            pos.is_in_check(ctx.enemy) && piece_attacks(pos, ctx.to, ctx.enemy_king(), ctx.us)
        }

        "has_own_piece_as_screen" => {
            if !ctx.on_same_line_as_king() {
                return false;
            }
            let between = ctx.pieces_between_attacker_and_king();
            if between.len() != 1 {
                return false;
            }
            let screen = pos.piece_at(between[0]);
            match (con.kind.as_deref().and_then(parse_kind), kind_of(screen)) {
                (Some(want), Some(actual)) => actual == want,
                _ => false,
            }
        }

        "screen_piece_adjacent_to_king" => {
            let Some(distance) = con.distance else {
                return false;
            };
            let between = ctx.pieces_between_attacker_and_king();
            if between.len() != 1 {
                return false;
            }
            chebyshev(between[0], ctx.enemy_king()) <= distance
        }

        "attacker_same_file_or_rank_as_king" => ctx.on_same_line_as_king(),
        "attacker_same_file_as_king" | "attacker_on_king_file" => {
            ctx.col() == col_of(ctx.enemy_king())
        }
        "no_piece_between_attacker_and_king" => {
            ctx.on_same_line_as_king() && ctx.pieces_between_attacker_and_king().is_empty()
        }

        // ---- 位置 ----
        "attacker_in_enemy_half" => in_enemy_half(ctx.us, ctx.row()),
        "attacker_outside_palace" => !xq_core::square::in_palace(ctx.enemy, ctx.col(), ctx.row()),
        "attacker_on_central_file" => ctx.col() == 4,
        "attacker_on_enemy_base_rank" => ctx.row() == enemy_base_row(ctx.enemy),
        "attacker_on_rank" => {
            let Some(offset) = con.row_offset_from_enemy_base else {
                return false;
            };
            let base = enemy_base_row(ctx.enemy) as i8;
            // 从对方底线往己方方向数 offset 行
            let step = if ctx.enemy == Color::Black { -1i8 } else { 1 };
            let want = base + step * offset as i8;
            (0..10).contains(&want) && ctx.row() as i8 == want
        }
        "attacker_adjacent_to_palace_corner" => {
            let Some(distance) = con.distance else {
                return false;
            };
            palace_corners(ctx.enemy)
                .iter()
                .any(|corner| chebyshev(ctx.to, *corner) <= distance)
        }
        "attacker_horizontal_distance_to_palace" => {
            let (Some(min), Some(max)) = (con.min, con.max) else {
                return false;
            };
            let d = horizontal_distance_to_palace(ctx.col());
            (min..=max).contains(&d)
        }
        "attacker_on_flank_rank" => {
            in_enemy_half(ctx.us, ctx.row())
                && ctx.col() != 4
                && horizontal_distance_to_palace(ctx.col()) <= 3
        }

        // ---- 己方子力分布 ----
        "own_piece_count_on_board" => {
            let (Some(kind), Some(min)) = (con.kind.as_deref().and_then(parse_kind), con.min)
            else {
                return false;
            };
            count_own_pieces(pos, ctx.us, kind) >= min as u32
        }
        "own_piece_count_on_line" => {
            let (Some(kind), Some(min)) = (con.kind.as_deref().and_then(parse_kind), con.min)
            else {
                return false;
            };
            let mut n = 0u32;
            for c in 0..9u8 {
                let piece = pos.piece_at(index(c, ctx.row()));
                if color_of(piece) == Some(ctx.us) && kind_of(piece) == Some(kind) {
                    n += 1;
                }
            }
            for r in 0..10u8 {
                let piece = pos.piece_at(index(ctx.col(), r));
                if color_of(piece) == Some(ctx.us) && kind_of(piece) == Some(kind) {
                    n += 1;
                }
            }
            n >= min as u32
        }
        "own_piece_count_on_file" => {
            let (Some(kind), Some(min)) = (con.kind.as_deref().and_then(parse_kind), con.min)
            else {
                return false;
            };
            let col = con.col.unwrap_or_else(|| ctx.col());
            if col >= 9 {
                return false;
            }
            let mut n = 0u32;
            for r in 0..10u8 {
                let piece = pos.piece_at(index(col, r));
                if color_of(piece) == Some(ctx.us) && kind_of(piece) == Some(kind) {
                    n += 1;
                }
            }
            n >= min as u32
        }
        "own_piece_on_enemy_base_rank" => {
            let (Some(kind), Some(min)) = (con.kind.as_deref().and_then(parse_kind), con.min)
            else {
                return false;
            };
            let row = enemy_base_row(ctx.enemy);
            let mut n = 0u32;
            for c in 0..9u8 {
                let piece = pos.piece_at(index(c, row));
                if color_of(piece) == Some(ctx.us) && kind_of(piece) == Some(kind) {
                    n += 1;
                }
            }
            n >= min as u32
        }
        "own_piece_on_king_file" => {
            let Some(min) = con.min else { return false };
            let col = col_of(ctx.enemy_king());
            let mut n = 0u32;
            for r in 0..10u8 {
                if color_of(pos.piece_at(index(col, r))) == Some(ctx.us) {
                    n += 1;
                }
            }
            n >= min as u32
        }
        "own_piece_near_king" => {
            let (Some(kind), Some(distance)) =
                (con.kind.as_deref().and_then(parse_kind), con.distance)
            else {
                return false;
            };
            let king = ctx.enemy_king();
            (0..BOARD_SIZE as u8).any(|sq| {
                let piece = pos.piece_at(sq);
                color_of(piece) == Some(ctx.us)
                    && kind_of(piece) == Some(kind)
                    && chebyshev(sq, king) <= distance
            })
        }
        "own_pieces_adjacent_to_palace" => {
            let (Some(min), Some(distance)) = (con.min, con.distance) else {
                return false;
            };
            let mut n = 0u32;
            for sq in 0..BOARD_SIZE as u8 {
                if color_of(pos.piece_at(sq)) != Some(ctx.us) {
                    continue;
                }
                if palace_corners(ctx.enemy)
                    .iter()
                    .any(|corner| chebyshev(sq, *corner) <= distance)
                {
                    n += 1;
                }
            }
            n >= min as u32
        }

        // ---- 将帅处境 ----
        "enemy_king_escape_blocked" => {
            let Some(min) = con.min else { return false };
            let n = ctx
                .king_escape_squares()
                .iter()
                .filter(|sq| pos.piece_at(**sq) != EMPTY)
                .count();
            n >= min as usize
        }
        "enemy_king_escape_blocked_by_own_pieces" => {
            let Some(min) = con.min else { return false };
            let n = ctx
                .king_escape_squares()
                .iter()
                .filter(|sq| color_of(pos.piece_at(**sq)) == Some(ctx.enemy))
                .count();
            n >= min as usize
        }
        "enemy_king_has_no_legal_move" => {
            // 只判将帅自身：九宫内四个方向的落点是否全部不可去
            ctx.king_escape_squares().iter().all(|sq| {
                let piece = pos.piece_at(*sq);
                color_of(piece) == Some(ctx.enemy) || pos.is_attacked(*sq, ctx.us)
            })
        }

        // ---- 将军能力 ----
        "attacker_checks_king_within_n_plies" => {
            // 保守近似：只判「本步之后己方是否已能将军」。多步推演需要再搜一层，
            // 与「≤ 5 ms」的本地延迟目标冲突；宁可少判（少一条棋形标签）也不误判。
            con.plies.unwrap_or(1) >= 1 && pos.is_in_check(ctx.enemy)
        }

        // ---- 吃子 ----
        "captured_piece_kind" => {
            if ctx.captured == EMPTY {
                return false;
            }
            match (
                con.kind.as_deref().and_then(parse_kind),
                kind_of(ctx.captured),
            ) {
                (Some(want), Some(actual)) => want == actual,
                _ => false,
            }
        }
        "captured_piece_in_enemy_palace" => {
            if ctx.captured == EMPTY {
                return false;
            }
            let (c, r) = (col_of(ctx.to), row_of(ctx.to));
            xq_core::square::in_palace(ctx.enemy, c, r)
        }

        // ---- 弃子 ----
        "attacker_is_sacrificed" => {
            let attacked = !attackers_of(pos, ctx.to, ctx.enemy).is_empty();
            let defended = is_defended(pos, ctx.to, ctx.us);
            attacked && !defended
        }

        // 未实现的谓词：保守返回 false（宁可漏判，不可误判）
        _ => false,
    }
}
