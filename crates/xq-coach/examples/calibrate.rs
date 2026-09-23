//! 讲解评价阈值的**标定工具**。
//!
//! # 为什么要标定
//!
//! `xq-coach` 的评价阈值（10 / 50 / 150 / 400 厘兵）是**照着经验画出来的刻度**，
//! 不是量出来的。阈值偏紧会把「同样好的棋」误报成「稍有问题」；
//! 偏松则会把真正的失误标成「最佳」—— 两者都会直接损害讲解的可信度。
//!
//! # 做法（docs/05 §9.2）
//!
//! ```text
//! 弱档引擎扮演「玩家」  →  走一步（带失误率，模拟人类水平）
//! 强档引擎扮演「裁判」  →  在**同一局面**上算出全部根着法评分
//! 分差 = 裁判最优分 − 实际着法分      ← 评价定级的唯一依据
//! ```
//!
//! 这正是产品里的真实链路：人类走子 → 引擎评判 → 讲解定级。
//!
//! # 用法
//!
//! ```bash
//! cargo run --release -p xq-coach --example calibrate -- 8
//! ```
//!
//! 参数是局数（默认 8）。**必须用 `--release`**，否则搜索慢一个数量级。

use std::time::Instant;

use xq_ai::{Difficulty, Engine, NeverStop, SearchLimits};
use xq_coach::MoveLevel;
use xq_coach::assess::{
    THRESHOLD_BEST, THRESHOLD_BLUNDER, THRESHOLD_DUBIOUS, THRESHOLD_GOOD, assess,
};
use xq_core::{Color, GameStatus, Position};

/// 扮演「玩家」的档位与深度 —— 带失误率，模拟人类水平的落子。
const PLAYER_LEVEL: Difficulty = Difficulty::L2;
/// 扮演「裁判」的搜索深度 —— 要比玩家强，否则评判本身就不准。
const JUDGE_DEPTH: u8 = 6;
/// 单局最多走多少半步。
const MAX_PLIES: usize = 60;

/// 扮演「玩家」的方式。
///
/// **这是整个标定里最关键的建模选择** —— 实测证明：换一个玩家模型，
/// 分差分布的形状会完全改变（见下面的实测对照）。用引擎当玩家会得到
/// 「90% 都是最优着法」的分布，但人类新手显然不是这样下棋的。
#[derive(Clone, Copy, PartialEq, Eq)]
enum PlayerModel {
    /// 弱档引擎（带失误率）。接近「有一点水平的人」。
    Engine,
    /// 全部合法着法里随机挑。代表「完全不会下棋」的下界。
    Random,
    /// 七成走引擎着法、三成随机。接近「新手」的粗糙近似。
    Mixed,
}

fn parse_player(s: &str) -> PlayerModel {
    match s {
        "random" => PlayerModel::Random,
        "mixed" => PlayerModel::Mixed,
        _ => PlayerModel::Engine,
    }
}

impl PlayerModel {
    fn label(self) -> &'static str {
        match self {
            PlayerModel::Engine => "弱档引擎（L2，带失误率）",
            PlayerModel::Random => "合法着法随机",
            PlayerModel::Mixed => "七成引擎 + 三成随机",
        }
    }
}

/// 按玩家模型选一步棋。
fn pick_move(
    model: PlayerModel,
    player: &mut Engine,
    pos: &mut Position,
    rng: &mut u64,
) -> Option<xq_core::Move> {
    // 简易 xorshift —— 与 xq-ai 内部同款，保证可复现
    let mut next = || {
        *rng ^= *rng << 13;
        *rng ^= *rng >> 7;
        *rng ^= *rng << 17;
        *rng
    };

    match model {
        PlayerModel::Engine => {
            player
                .search_with(pos, SearchLimits::depth(4), &NeverStop)
                .best_move
        }
        PlayerModel::Random => {
            let moves = pos.legal_moves();
            if moves.is_empty() {
                None
            } else {
                Some(moves[(next() % moves.len() as u64) as usize])
            }
        }
        PlayerModel::Mixed => {
            if next() % 10 < 3 {
                let moves = pos.legal_moves();
                if moves.is_empty() {
                    return None;
                }
                Some(moves[(next() % moves.len() as u64) as usize])
            } else {
                player
                    .search_with(pos, SearchLimits::depth(4), &NeverStop)
                    .best_move
            }
        }
    }
}

fn main() {
    let games: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let model = parse_player(
        &std::env::args()
            .nth(2)
            .unwrap_or_else(|| "engine".to_string()),
    );

    println!("讲解评价阈值标定");
    println!("────────────────────────────────────────────");
    println!("  玩家模型 = {}", model.label());
    println!("  裁判     = 深度 {JUDGE_DEPTH} 的完整搜索（同一局面重新评判）");
    println!("  局数     = {games}，单局上限 {MAX_PLIES} 半步");
    println!();

    let started = Instant::now();
    let mut losses: Vec<i32> = Vec::new();
    let mut levels: [usize; 5] = [0; 5];

    for game in 0..games {
        let mut player = Engine::new(game as u64 * 7919 + 13);
        player.set_difficulty(PLAYER_LEVEL);
        player.new_game();

        let mut judge = Engine::new(0xCAFE_F00D + game as u64);
        judge.new_game();

        let mut pos = Position::startpos();
        let mut rng = 0x9E37_79B9_7F4A_7C15u64 ^ (game as u64 + 1);

        for _ in 0..MAX_PLIES {
            if pos.status().is_over() {
                break;
            }

            // ① 玩家走子
            let Some(played) = pick_move(model, &mut player, &mut pos, &mut rng) else {
                break;
            };

            // ② 裁判在同一局面上评判
            let verdict = judge.search_with(&mut pos, SearchLimits::depth(JUDGE_DEPTH), &NeverStop);

            // ③ 分差
            let played_mates = mates_after(&pos, played);
            let best_mates = verdict
                .best_move
                .map(|best| mates_after(&pos, best))
                .unwrap_or(false);
            let a = assess(&verdict.root_moves, played, played_mates, best_mates);

            losses.push(a.score_loss);
            levels[level_index(a.level)] += 1;

            if pos.make_move(played).is_err() {
                break;
            }
        }

        print!("\r  已完成 {}/{} 局…", game + 1, games);
    }
    println!(
        "\r  标定完成，用时 {:.1} 秒      ",
        started.elapsed().as_secs_f32()
    );
    println!();

    report(&losses, &levels);
}

fn level_index(level: MoveLevel) -> usize {
    match level {
        MoveLevel::Best => 0,
        MoveLevel::Good => 1,
        MoveLevel::Dubious => 2,
        MoveLevel::Blunder => 3,
        MoveLevel::Missed => 4,
    }
}

/// 走完这一步之后，对方是否被将死。
fn mates_after(pos: &Position, mv: xq_core::Move) -> bool {
    let mut after = pos.clone();
    if after.make_move(mv).is_err() {
        return false;
    }
    let mut probe = after;
    matches!(
        probe.status(),
        GameStatus::Checkmate {
            loser: l
        } if l == pos.side_to_move().opponent()
    )
}

fn report(losses: &[i32], levels: &[usize; 5]) {
    if losses.is_empty() {
        println!("没有采集到样本（检查搜索是否正常）。");
        return;
    }

    let total = losses.len();
    let mut sorted = losses.to_vec();
    sorted.sort_unstable();

    let sum: i64 = losses.iter().map(|v| *v as i64).sum();
    let mean = sum as f64 / total as f64;
    let percentile = |p: f64| -> i32 {
        let idx = ((total as f64 - 1.0) * p).round() as usize;
        sorted[idx.min(total - 1)]
    };

    println!("分差分布（{total} 个样本，单位：厘兵，100 = 一个兵）");
    println!("────────────────────────────────────────────");
    println!(
        "  均值 {mean:.1}  ·  中位数 {}  ·  p90 {}  ·  p99 {}  ·  最大 {}",
        percentile(0.5),
        percentile(0.9),
        percentile(0.99),
        sorted[total - 1]
    );
    println!();

    // ---- 分档直方图 ----
    let buckets: [(i32, i32, &str); 6] = [
        (0, 0, "  = 0（走了最优）"),
        (1, THRESHOLD_BEST, "  1 ~ 10"),
        (THRESHOLD_BEST + 1, THRESHOLD_GOOD, "  11 ~ 50"),
        (THRESHOLD_GOOD + 1, THRESHOLD_DUBIOUS, "  51 ~ 150"),
        (THRESHOLD_DUBIOUS + 1, THRESHOLD_BLUNDER, "  151 ~ 400"),
        (THRESHOLD_BLUNDER + 1, i32::MAX, "  > 400"),
    ];
    for (lo, hi, label) in buckets {
        let n = losses.iter().filter(|v| **v >= lo && **v <= hi).count();
        let pct = n as f64 / total as f64 * 100.0;
        let bar = "█".repeat(((pct / 2.0) as usize).min(40));
        println!("{label:16} {n:5}  {pct:5.1}%  {bar}");
    }
    println!();

    // ---- 等级分布 vs 设计预期 ----
    println!("等级分布对比（docs/05 §9.2 的经验预期）");
    println!("────────────────────────────────────────────");
    let names = [
        "最佳 Best",
        "不错 Good",
        "稍有问题 Dubious",
        "明显失误 Blunder",
        "严重漏着 Missed",
    ];
    let expected = [
        (60.0, 70.0),
        (15.0, 20.0),
        (8.0, 12.0),
        (3.0, 6.0),
        (0.0, 1.0),
    ];
    println!("  {:<20} {:>8}  {:>10}  判定", "等级", "实测", "预期区间");
    for i in 0..5 {
        let pct = levels[i] as f64 / total as f64 * 100.0;
        let (lo, hi) = expected[i];
        let verdict = if pct < lo {
            "偏低"
        } else if pct > hi {
            "偏高"
        } else {
            "相符"
        };
        println!(
            "  {:<20} {:>7.1}%  {:>4.0}~{:<4.0}%  {}",
            names[i], pct, lo, hi, verdict
        );
    }
    println!();
    println!("  注：预期区间是 docs/05 §9.2 标注的「待验证假设」，");
    println!("      本次实测的意义正是把它们换成真实数字。");
    let _ = Color::Red;
}
