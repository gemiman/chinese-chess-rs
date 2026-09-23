//! 评估函数：子力价值 + 位置价值（PST）+ 阶段权重混合 + 将帅安全。
//!
//! # 分数单位与视角
//!
//! 单位是**厘兵**（1 个兵 = 100），与 [docs/01 §4.3](../../../docs/01-需求规格说明书.md)
//! 的评价分档阈值（10 / 50 / 150 / 400）同量纲。
//!
//! **视角统一为红方**（红优为正）。Alpha-Beta 中通过取负转换到走子方视角。
//! 这条约定必须严格统一 —— 弄反了就会出现「引擎帮对手走棋」的经典 bug。
//!
//! # PST 为什么用规则表而不是裸数字
//!
//! [docs/04 §4.3](../../../docs/04-AI引擎设计.md) 要求把位置价值**按规则形式化表达**，
//! 实现时展开成表。本模块正是这么做的：规则写在 [`pst_square`] 里（可读、可审查、
//! 可调优），表由 `const fn` 在编译期展开，运行时零成本。
//!
//! 所有表都是**红方视角**（`row 0` 是红方底线）。黑方查询时做行镜像
//! （`row' = 9 - row`）后查同一张表 —— 只维护一份表，从根上消除两份表不对称的 bug。

use xq_core::piece::{EMPTY, PieceKind, color_of, kind_of};
use xq_core::square::{BOARD_SIZE, col_of};
use xq_core::{Color, Position};

// ---------------------------------------------------------------- 子力价值

/// 车的基准价值。
pub const VALUE_CHARIOT: i32 = 900;
/// 炮在**开局**的价值（残局会衰减，见 [`material_value`]）。
pub const VALUE_CANNON_OPENING: i32 = 450;
/// 炮在**残局**的价值。
pub const VALUE_CANNON_ENDGAME: i32 = 380;
/// 马在**开局**的价值。
pub const VALUE_HORSE_OPENING: i32 = 400;
/// 马在**残局**的价值（残局无阻挡，马威大增）。
pub const VALUE_HORSE_ENDGAME: i32 = 480;
/// 仕 / 士。
pub const VALUE_ADVISOR: i32 = 200;
/// 相 / 象。
pub const VALUE_ELEPHANT: i32 = 200;
/// 兵 / 卒（未过河）。
pub const VALUE_PAWN: i32 = 100;

/// 开局满盘时的「阶段权重总量」。
///
/// 权重：车 6、炮 4、马 4、兵 2，仕/相/将 0（不参与阶段判断）。
/// 单方 = 2×6 + 2×4 + 2×4 + 5×2 = 38；双方 = **76**。
///
/// `phase_weight_initial_is_76` 这个测试会验证它和初始局面一致 ——
/// 一旦有人改了权重却忘了改这个常量，测试会立刻报警。
pub const FULL_PHASE_WEIGHT: i32 = 76;

/// 阶段系数的定点刻度（避免浮点，保证跨平台结果完全一致）。
const PHASE_SCALE: i32 = 256;

/// 某棋子在阶段判断中的权重。
#[inline]
const fn phase_weight(kind: PieceKind) -> i32 {
    match kind {
        PieceKind::Chariot => 6,
        PieceKind::Cannon | PieceKind::Horse => 4,
        PieceKind::Pawn => 2,
        // 仕/相/将不参与：它们数量恒定或变化不影响阶段
        PieceKind::Advisor | PieceKind::Elephant | PieceKind::King => 0,
    }
}

/// 阶段系数：`PHASE_SCALE` = 纯开局，0 = 纯残局。
pub fn phase_factor(pos: &Position) -> i32 {
    let mut weight = 0;
    for idx in 0..BOARD_SIZE as u8 {
        let piece = pos.piece_at(idx);
        if piece == EMPTY {
            continue;
        }
        if let Some(kind) = kind_of(piece) {
            weight += phase_weight(kind);
        }
    }
    ((weight * PHASE_SCALE) / FULL_PHASE_WEIGHT).clamp(0, PHASE_SCALE)
}

/// 按阶段插值的子力价值。
fn material_value(kind: PieceKind, phase: i32) -> i32 {
    match kind {
        PieceKind::Chariot => VALUE_CHARIOT,
        // 炮：开局强、残局弱
        PieceKind::Cannon => {
            VALUE_CANNON_ENDGAME
                + (VALUE_CANNON_OPENING - VALUE_CANNON_ENDGAME) * phase / PHASE_SCALE
        }
        // 马：开局弱、残局强
        PieceKind::Horse => {
            VALUE_HORSE_OPENING
                + (VALUE_HORSE_ENDGAME - VALUE_HORSE_OPENING) * (PHASE_SCALE - phase) / PHASE_SCALE
        }
        PieceKind::Advisor => VALUE_ADVISOR,
        PieceKind::Elephant => VALUE_ELEPHANT,
        PieceKind::Pawn => VALUE_PAWN,
        // 将帅不参与常规评估：被吃由搜索的杀棋分数处理
        PieceKind::King => 0,
    }
}

// ---------------------------------------------------------------- PST

/// 兵 / 卒的位置价值。过河是质变。
const fn pawn_pst(col: usize, row: usize) -> i16 {
    let mut v: i16 = match row {
        0..=4 => 0, // 未过河
        5 => 20,    // 刚过河
        6 => 40,
        7 => 55,
        8 => 35,  // 接近底线，横移能力受限，价值回落
        _ => -50, // row 9：老兵，几乎失去作用（基准 100 → 实值约 50）
    };
    if col == 4 {
        v += 15; // 中路：对将帅的直接威胁更大
    }
    if col == 0 || col == 8 {
        v -= 10; // 边路：作用面窄
    }
    v
}

/// 马的位置价值。位置敏感度最高的棋子。
const fn horse_pst(col: usize, row: usize) -> i16 {
    let mut v: i16 = 0;
    if col >= 3 && col <= 5 && row >= 3 && row <= 6 {
        v += 25; // 中心区域
    }
    if col == 0 || col == 8 {
        v -= 20; // 边线：机动性大减
    }
    if row == 0 || row == 9 {
        v -= 15; // 底线：出路少
    }
    if row == 4 || row == 5 {
        v += 10; // 己方河界附近，便于过河
    }
    v
}

/// 炮的位置价值。依赖炮架，故位置敏感度低于马。
const fn cannon_pst(col: usize, _row: usize) -> i16 {
    let mut v: i16 = 0;
    if col == 4 {
        v += 20; // 中路：潜在炮架多
    }
    v
}

/// 车的位置价值。
const fn chariot_pst(col: usize, row: usize) -> i16 {
    let mut v: i16 = 0;
    if row >= 5 {
        v += 15; // 深入对方半场
    }
    if col == 4 {
        v += 10; // 中路
    }
    v
}

/// 仕 / 士：在原位保持防守结构完整。
const fn advisor_pst(col: usize, row: usize) -> i16 {
    // 红方原始位置 d0(col 3) 与 f0(col 5)
    if row == 0 && (col == 3 || col == 5) {
        10
    } else {
        -5
    }
}

/// 相 / 象：同上。
const fn elephant_pst(col: usize, row: usize) -> i16 {
    // 红方原始位置 c0(col 2) 与 g0(col 6)
    if row == 0 && (col == 2 || col == 6) {
        10
    } else {
        -5
    }
}

/// 单格位置价值（红方视角）。
const fn pst_square(kind: PieceKind, col: usize, row: usize) -> i16 {
    match kind {
        PieceKind::Pawn => pawn_pst(col, row),
        PieceKind::Horse => horse_pst(col, row),
        PieceKind::Cannon => cannon_pst(col, row),
        PieceKind::Chariot => chariot_pst(col, row),
        PieceKind::Advisor => advisor_pst(col, row),
        PieceKind::Elephant => elephant_pst(col, row),
        PieceKind::King => 0,
    }
}

/// 在编译期把规则展开成 90 格表。
const fn build_pst(kind: PieceKind) -> [i16; BOARD_SIZE] {
    let mut table = [0i16; BOARD_SIZE];
    let mut row = 0;
    while row < 10 {
        let mut col = 0;
        while col < 9 {
            table[row * 9 + col] = pst_square(kind, col, row);
            col += 1;
        }
        row += 1;
    }
    table
}

static PST_PAWN: [i16; BOARD_SIZE] = build_pst(PieceKind::Pawn);
static PST_HORSE: [i16; BOARD_SIZE] = build_pst(PieceKind::Horse);
static PST_CANNON: [i16; BOARD_SIZE] = build_pst(PieceKind::Cannon);
static PST_CHARIOT: [i16; BOARD_SIZE] = build_pst(PieceKind::Chariot);
static PST_ADVISOR: [i16; BOARD_SIZE] = build_pst(PieceKind::Advisor);
static PST_ELEPHANT: [i16; BOARD_SIZE] = build_pst(PieceKind::Elephant);

/// 将帅全零表。
///
/// 不能让 [`pst`] 对将帅返回别的表 —— 那样一旦在评估里用到将帅的位置价值，
/// 就会静默地拿到「兵的 PST」，产生一个极难定位的偏置。
static PST_KING: [i16; BOARD_SIZE] = [0; BOARD_SIZE];

/// 取某棋子的位置价值表（红方视角）。
pub fn pst(kind: PieceKind) -> &'static [i16; BOARD_SIZE] {
    match kind {
        PieceKind::Pawn => &PST_PAWN,
        PieceKind::Horse => &PST_HORSE,
        PieceKind::Cannon => &PST_CANNON,
        PieceKind::Chariot => &PST_CHARIOT,
        PieceKind::Advisor => &PST_ADVISOR,
        PieceKind::Elephant => &PST_ELEPHANT,
        PieceKind::King => &PST_KING,
    }
}

/// 红方视角索引 → 黑方视角索引（行镜像）。
#[inline]
pub const fn mirror_index(idx: u8) -> u8 {
    let col = idx % 9;
    let row = idx / 9;
    (9 - row) * 9 + col
}

// ---------------------------------------------------------------- 评估

/// 单方在一次遍历中累积的统计量。
#[derive(Default, Clone, Copy)]
struct SideStat {
    chariots: i32,
    /// 炮数与马数分开记：只有这两种棋子的价值随阶段插值
    horses: i32,
    cannons: i32,
    advisors: i32,
    elephants: i32,
    pawns: i32,
    /// 位置价值合计（已按红方视角查表）
    pst_sum: i32,
    /// 车的所在格（用于算开放线加分）。一方最多两车，4 格足够。
    rook_squares: [u8; 4],
    rook_count: usize,
}

impl SideStat {
    #[inline]
    fn add_rook(&mut self, idx: u8) {
        if self.rook_count < self.rook_squares.len() {
            self.rook_squares[self.rook_count] = idx;
            self.rook_count += 1;
        }
    }

    /// 子力总分（含炮 / 马的阶段插值）。
    fn material(&self, phase: i32) -> i32 {
        self.chariots * VALUE_CHARIOT
            + self.cannons * material_value(PieceKind::Cannon, phase)
            + self.horses * material_value(PieceKind::Horse, phase)
            + self.advisors * VALUE_ADVISOR
            + self.elephants * VALUE_ELEPHANT
            + self.pawns * VALUE_PAWN
    }

    /// 将帅安全：目前只实现「防守子力完整性」这一项。
    ///
    /// > 首版**刻意不做**「九宫正面被对方车/炮直线瞄准」与「闷宫风险」两项：
    /// > 前者的绝大多数情形其实就是**将军**，搜索会在下一层处理，重复计分反而
    /// > 会让评估函数与搜索结论冲突；后者需要判断「将帅退路是否被己方兵卒堵死」，
    /// > 属于需要实测调优的启发项。两项都留到参数标定阶段（docs/04 §8.2）再加。
    fn king_safety(&self) -> i32 {
        let mut score = 0;
        if self.advisors == 2 && self.elephants == 2 {
            score += 30; // 防守结构完整
        }
        score - (2 - self.advisors.min(2)) * 25 - (2 - self.elephants.min(2)) * 25
    }

    /// 车的开放线加分：双方都无兵 = 开放线 20；仅对方有兵 = 半开放线 10。
    fn rook_file_bonus(&self, own_pawns: &[bool; 9], enemy_pawns: &[bool; 9]) -> i32 {
        let mut sum = 0;
        for i in 0..self.rook_count {
            let col = col_of(self.rook_squares[i]) as usize;
            sum += match (own_pawns[col], enemy_pawns[col]) {
                (false, false) => 20,
                (false, true) => 10,
                _ => 0,
            };
        }
        sum
    }
}

/// 评估函数。**返回红方视角**的绝对分（红优为正）。
///
/// # 性能设计：单趟遍历
///
/// 每片叶子都要调用评估，它是搜索里最热的函数。初版写成「先扫一遍算阶段系数、
/// 再扫一遍算子力与位置、遇到每辆车再扫一遍该列找兵」—— 每片叶子要做 3 次以上
/// 全盘扫描，实测节点速率只有 32K/s。改成**单趟遍历**（顺便把「哪几列有兵」
/// 也在同一趟里记下来）后，同样的搜索能多跑近一倍的节点。
///
/// 这不是过早优化：静态搜索会把叶子数量放大一个数量级，评估慢一倍，棋力就实打实掉一档。
pub fn evaluate(pos: &Position) -> i32 {
    let mut weight = 0i32;
    let mut red = SideStat::default();
    let mut black = SideStat::default();
    let mut red_pawn_cols = [false; 9];
    let mut black_pawn_cols = [false; 9];

    for idx in 0..BOARD_SIZE as u8 {
        let piece = pos.piece_at(idx);
        if piece == EMPTY {
            continue;
        }
        let Some(color) = color_of(piece) else {
            continue;
        };
        let Some(kind) = kind_of(piece) else { continue };

        weight += phase_weight(kind);

        let is_red = color == Color::Red;
        // 黑方换算到红方视角查表
        let table_idx = if is_red { idx } else { mirror_index(idx) };
        let stat = if is_red { &mut red } else { &mut black };
        stat.pst_sum += pst(kind)[table_idx as usize] as i32;

        match kind {
            PieceKind::Chariot => {
                stat.chariots += 1;
                stat.add_rook(idx);
            }
            PieceKind::Cannon => stat.cannons += 1,
            PieceKind::Horse => stat.horses += 1,
            PieceKind::Advisor => stat.advisors += 1,
            PieceKind::Elephant => stat.elephants += 1,
            PieceKind::Pawn => {
                stat.pawns += 1;
                let col = col_of(idx) as usize;
                if is_red {
                    red_pawn_cols[col] = true;
                } else {
                    black_pawn_cols[col] = true;
                }
            }
            PieceKind::King => {}
        }
    }

    let phase = ((weight * PHASE_SCALE) / FULL_PHASE_WEIGHT).clamp(0, PHASE_SCALE);

    let red_total = red.material(phase)
        + red.pst_sum
        + red.king_safety()
        + red.rook_file_bonus(&red_pawn_cols, &black_pawn_cols);
    let black_total = black.material(phase)
        + black.pst_sum
        + black.king_safety()
        + black.rook_file_bonus(&black_pawn_cols, &red_pawn_cols);

    let score = red_total - black_total;

    debug_assert!(
        score.abs() < 20_000,
        "评估分 {score} 超出预期范围，会与杀棋分数区间冲突"
    );
    score
}

/// 走子方视角的评估分。
#[inline]
pub fn evaluate_for(pos: &Position, color: Color) -> i32 {
    let red_view = evaluate(pos);
    match color {
        Color::Red => red_view,
        Color::Black => -red_view,
    }
}

/// 便捷入口：当前走子方视角。
#[inline]
pub fn evaluate_side_to_move(pos: &Position) -> i32 {
    evaluate_for(pos, pos.side_to_move())
}

#[cfg(test)]
mod tests {
    use super::*;
    use xq_core::square::from_iccs;

    #[test]
    fn phase_weight_initial_is_76() {
        let pos = Position::startpos();
        let mut weight = 0;
        for idx in 0..BOARD_SIZE as u8 {
            let piece = pos.piece_at(idx);
            if let Some(kind) = kind_of(piece) {
                weight += phase_weight(kind);
            }
        }
        assert_eq!(weight, FULL_PHASE_WEIGHT, "初始局面的阶段权重应恰为常量值");
        assert_eq!(phase_factor(&pos), PHASE_SCALE, "初始局面应是纯开局");
    }

    #[test]
    fn mirror_is_involution() {
        for idx in 0..BOARD_SIZE as u8 {
            assert_eq!(mirror_index(mirror_index(idx)), idx);
        }
    }

    /// 初始局面必须完全对称 → 评估分恰为 0。
    #[test]
    fn startpos_is_balanced() {
        assert_eq!(evaluate(&Position::startpos()), 0);
    }

    /// **镜像对称性**：把局面上下翻转并交换红黑，评估分必须取反。
    ///
    /// 这是最高效的 bug 探测器 —— 一次能捕获 PST 表不对称、阶段权重算错、
    /// 某个棋子价值漏加等一大批问题。
    #[test]
    fn mirrored_position_evaluates_to_negation() {
        // 构造一个明显不对称的局面
        let pieces = [
            (Color::Red, PieceKind::King, "e0"),
            (Color::Red, PieceKind::Chariot, "b2"),
            (Color::Red, PieceKind::Horse, "e5"),
            (Color::Red, PieceKind::Pawn, "c6"),
            (Color::Black, PieceKind::King, "d9"),
            (Color::Black, PieceKind::Cannon, "g4"),
            (Color::Black, PieceKind::Advisor, "f9"),
        ];
        let pos = xq_core::position_from_pieces(&pieces, Color::Red).unwrap();

        // 镜像：坐标行翻转（row → 9-row），颜色互换
        let mirrored: Vec<(Color, PieceKind, String)> = pieces
            .iter()
            .map(|(color, kind, coord)| {
                let idx = from_iccs(coord).unwrap();
                let m = mirror_index(idx);
                let text = xq_core::to_iccs(m);
                (color.opponent(), *kind, text)
            })
            .collect();
        let borrowed: Vec<(Color, PieceKind, &str)> = mirrored
            .iter()
            .map(|(c, k, s)| (*c, *k, s.as_str()))
            .collect();
        let mirrored_pos = xq_core::position_from_pieces(&borrowed, Color::Black).unwrap();

        assert_eq!(
            evaluate(&mirrored_pos),
            -evaluate(&pos),
            "镜像局面的评估分应严格取反"
        );
    }

    #[test]
    fn material_advantage_is_positive_for_red() {
        // 红方多一个车
        let pos = xq_core::position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Chariot, "a0"),
                (Color::Black, PieceKind::King, "e9"),
            ],
            Color::Red,
        )
        .unwrap();
        assert!(evaluate(&pos) > 400, "红方多一车应为明显正分");
    }

    #[test]
    fn crossed_pawn_worth_more_than_home_pawn() {
        let home = xq_core::position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Pawn, "e3"),
                (Color::Black, PieceKind::King, "e9"),
            ],
            Color::Red,
        )
        .unwrap();
        let crossed = xq_core::position_from_pieces(
            &[
                (Color::Red, PieceKind::King, "e0"),
                (Color::Red, PieceKind::Pawn, "e7"),
                (Color::Black, PieceKind::King, "e9"),
            ],
            Color::Red,
        )
        .unwrap();
        assert!(
            evaluate(&crossed) > evaluate(&home) + 40,
            "过河的兵应明显更值钱"
        );
    }

    #[test]
    fn pawn_pst_matches_documented_rules() {
        // 文档 §4.3 的规则：row 5 → +20、row 6 → +40、row 7 → +55、row 8 → +35、row 9 → 老兵
        // 用 col 2（既非中路也非边路）取基准值，避免附加项干扰
        assert_eq!(pawn_pst(2, 4), 0);
        assert_eq!(pawn_pst(2, 5), 20);
        assert_eq!(pawn_pst(2, 6), 40);
        assert_eq!(pawn_pst(2, 7), 55);
        assert_eq!(pawn_pst(2, 8), 35);
        assert_eq!(pawn_pst(2, 9), -50);
        // 中路 +15、边路 −10
        assert_eq!(pawn_pst(4, 6), 55);
        assert_eq!(pawn_pst(8, 6), 30);
        assert_eq!(pawn_pst(0, 4), -10);
    }

    #[test]
    fn cannon_value_decays_and_horse_grows_in_endgame() {
        let opening = material_value(PieceKind::Cannon, PHASE_SCALE);
        let endgame = material_value(PieceKind::Cannon, 0);
        assert!(opening > endgame, "炮在残局应贬值");

        let horse_opening = material_value(PieceKind::Horse, PHASE_SCALE);
        let horse_endgame = material_value(PieceKind::Horse, 0);
        assert!(horse_endgame > horse_opening, "马在残局应增值");
    }

    #[test]
    fn square_index_helper_is_consistent() {
        // 顺带验证 eval 用到的坐标换算与 xq-core 一致
        let idx = from_iccs("c6").unwrap();
        assert_eq!(col_of(idx), 2);
        assert_eq!(xq_core::square::row_of(idx), 6);
    }
}
