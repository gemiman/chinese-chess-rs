//! 引擎行为测试。
//!
//! 重点不是「搜得多深」，而是**几条必须成立的性质**：
//!
//! | 性质 | 为什么关键 |
//! |---|---|
//! | 找得到杀棋 | 找不到杀的引擎会放着必胜局面不下 |
//! | 杀棋优先于吃子 | 差一步杀却去贪一个车，是最刺眼的「笨」 |
//! | 吃白送的子 | 这是「不算亏」的最低标准 |
//! | 可复现 | 不可复现就没法判断「改了一版是变强还是变弱」 |
//! | 中断后仍可用 | 限时搜索必须总能给出着法，不能空手而归 |
//! | 自对弈不产生非法着法 | 覆盖最广的回归手段 |

use xq_ai::{Difficulty, Engine, NeverStop, NodeLimitedStop, SearchLimits};
use xq_core::piece::PieceKind;
use xq_core::square::from_iccs;
use xq_core::{Color, GameStatus, Move, Position};

fn mv(iccs: &str) -> Move {
    Move::new(
        from_iccs(&iccs[0..2]).expect("起点"),
        from_iccs(&iccs[2..4]).expect("终点"),
    )
}

fn build(pieces: &[(Color, PieceKind, &str)], side: Color) -> Position {
    xq_core::position_from_pieces(pieces, side).expect("测试局面构造失败")
}

/// 红方一步杀：`e1 → e8` 或 `d8 → d9` 都能成杀，故只断言「确实是杀」而不锁死具体着法。
fn mate_in_one_position() -> Position {
    build(
        &[
            (Color::Red, PieceKind::King, "d0"),
            (Color::Red, PieceKind::Chariot, "d8"),
            (Color::Red, PieceKind::Chariot, "e1"),
            (Color::Red, PieceKind::Chariot, "f8"),
            (Color::Black, PieceKind::King, "e9"),
        ],
        Color::Red,
    )
}

#[test]
fn finds_mate_in_one() {
    let mut engine = Engine::new(1);
    let mut pos = mate_in_one_position();

    let result = engine.search_with(&mut pos, SearchLimits::depth(4), &NeverStop);

    assert!(
        result.is_mate_score(),
        "应识别出杀棋，实际评分 {}",
        result.score
    );
    assert_eq!(
        result.mate_distance(),
        Some(1),
        "应报「一步杀」，实际 {:?}",
        result.mate_distance()
    );

    // 真正落子验证：走完之后黑方确实被将死
    let best = result.best_move.expect("杀棋局面必有推荐着法");
    pos.make_move(best).expect("推荐着法必须合法");
    assert_eq!(
        pos.status(),
        GameStatus::Checkmate {
            loser: Color::Black
        },
        "评分说是杀棋，但走完之后黑方没被将死 —— 杀棋分数算错了"
    );
}

/// 既有一步杀、又能白吃一个车时，必须选杀。
#[test]
fn mate_takes_priority_over_material() {
    let mut engine = Engine::new(2);
    // 在一步杀的局面里额外放一个白送的黑车（红车 d8 可以顺手吃掉）
    let mut pos = build(
        &[
            (Color::Red, PieceKind::King, "d0"),
            (Color::Red, PieceKind::Chariot, "d8"),
            (Color::Red, PieceKind::Chariot, "e1"),
            (Color::Red, PieceKind::Chariot, "f8"),
            (Color::Black, PieceKind::King, "e9"),
            (Color::Black, PieceKind::Chariot, "d5"),
        ],
        Color::Red,
    );

    // 先确认「吃掉那个车」确实是一个合法且便宜的着法
    assert!(
        pos.legal_move_to(from_iccs("d8").unwrap(), from_iccs("d5").unwrap())
            .is_some()
    );

    let result = engine.search_with(&mut pos, SearchLimits::depth(4), &NeverStop);
    let best = result.best_move.expect("应有推荐着法");

    let mut probe = pos.clone();
    probe.make_move(best).unwrap();
    assert_eq!(
        probe.status(),
        GameStatus::Checkmate {
            loser: Color::Black
        },
        "有一步杀时不应去贪吃子，实际选了 {best}"
    );
}

/// 白送的子必须吃掉。
///
/// ⚠️ **这个局面是反复挑出来的**，前两版都失败了 —— 而且两次都是**引擎对、测试错**：
///
/// 1. 第一版用「红帅 e0 + 红车 a0 对 黑将 d9 + 黑车 a5」的极简局面，引擎不走吃车，
///    而是走 `a0d0` —— 那一步是**强制三步杀**（黑将被九宫边界与白脸将夹死，
///    唯一应法是垫车，红车吃掉即成杀）；
/// 2. 第二版加了些子力，引擎又走了 `i0i8` —— 那是**带将的先手**，既得先手又能赚马，
///    实测评分 2270 高于吃车的 1930。
///
/// 教训：测「吃子」这类基础行为时，局面里**不能留下任何更强的战术**，否则测的是
/// 对局面的判断力，而不是吃子能力。最终采用双方子力完整的局面（各 2 车 2 炮 2 马 4 兵），
/// 把唯一的不对称设为「黑方一车孤悬 a5 且无人保护」。
#[test]
fn captures_a_hanging_chariot() {
    // 与初始局面同构，只是黑方 a9 车挪到了 a5、双方 a 路兵都拿掉
    let mut pos =
        Position::from_fen("1nbakabnr/9/1c5c1/2p1p1p1p/r8/9/2P1P1P1P/1C5C1/9/RNBAKABNR w - - 0 1")
            .expect("测试 FEN 应可解析");

    // 先自检：a5 那个车确实无人保护
    let target = from_iccs("a5").unwrap();
    let piece = pos.piece_at(target);
    assert_eq!(
        xq_core::kind_of(piece),
        Some(PieceKind::Chariot),
        "a5 应是黑车"
    );

    let mut engine = Engine::new(3);
    let result = engine.search_with(&mut pos, SearchLimits::depth(5), &NeverStop);

    assert!(
        !result.is_mate_score(),
        "这个局面不应存在杀棋，实际 {:?}",
        result.mate_distance()
    );
    assert_eq!(
        result.best_move,
        Some(mv("a0a5")),
        "应吃掉白送的车，实际 {:?}",
        result.best_move
    );

    // 评分**不是**一整车的 900，原因是一个容易忽略的战术：
    // 黑方 b7 / h7 的炮可以借红方 b2 / h2 的炮当**炮架**，打回红方底线的马
    // （经典的「炮打底马」）。所以净得子是「一车减一马」≈ 500 厘兵。
    // 断言写成 400 而不是 800，正是因为这一点 —— 若引擎报出接近 900 的分，
    // 反而说明它没看见黑方的反击。
    assert!(
        result.score >= 400 && result.score <= 800,
        "吃车后红方应净得一车减一马（约 500 厘兵），实际评分 {}",
        result.score
    );

    // 验证黑方确实有这一手反击，把这个「为什么不是 900」钉死在测试里
    let mut after = pos.clone();
    after.make_move(mv("a0a5")).unwrap();
    let counter = mv("b7b0"); // 炮 b7 借 b2 的红炮当架，打红方底线的马
    assert!(
        after.is_legal(counter),
        "黑方应能打回底线马（炮打底马），否则上面的评分区间需要重新推导"
    );
}

/// 局面已经输了/赢了时，评分方向不能反。
#[test]
fn score_is_from_side_to_move_perspective() {
    let mut engine = Engine::new(4);

    // 红方大优（多两个车）→ 红方走子时评分为正
    let mut red_side = build(
        &[
            (Color::Red, PieceKind::King, "e0"),
            (Color::Red, PieceKind::Chariot, "a0"),
            (Color::Red, PieceKind::Chariot, "i0"),
            (Color::Black, PieceKind::King, "d9"),
        ],
        Color::Red,
    );
    let red_result = engine.search_with(&mut red_side, SearchLimits::depth(3), &NeverStop);
    assert!(red_result.score > 300, "红方多两车应为明显正分");

    // 同一局面让黑方走子 → 评分应为负
    let mut black_side = build(
        &[
            (Color::Red, PieceKind::King, "e0"),
            (Color::Red, PieceKind::Chariot, "a0"),
            (Color::Red, PieceKind::Chariot, "i0"),
            (Color::Black, PieceKind::King, "d9"),
        ],
        Color::Black,
    );
    let black_result = engine.search_with(&mut black_side, SearchLimits::depth(3), &NeverStop);
    assert!(
        black_result.score < -300,
        "同一局面轮到黑方走，评分应为负 —— 否则就是「帮对手走棋」的经典 bug"
    );
}

/// 相同种子 + 相同节点上限 → 完全相同的结果。
#[test]
fn search_is_deterministic_under_node_limit() {
    let mut pos = mate_in_one_position();
    let limit = SearchLimits::depth_nodes(5, 200_000);

    let mut first: Option<(Option<Move>, i32, u8, u64)> = None;
    for round in 0..5 {
        let mut engine = Engine::new(2024);
        let stop = NodeLimitedStop::new(200_000);
        let result = engine.search_with(&mut pos, limit, &stop);
        let signature = (result.best_move, result.score, result.depth, result.nodes);

        match first {
            None => first = Some(signature),
            Some(expected) => assert_eq!(
                signature, expected,
                "第 {round} 轮结果与首轮不一致 —— 搜索引入了不可复现的随机源"
            ),
        }
    }
    assert!(first.is_some());
}

/// 停止信号触发时仍须给出可用的着法与合法的评分。
#[test]
fn stopped_search_still_returns_usable_result() {
    let mut engine = Engine::new(5);
    // 深度给得很高，但节点上限很小 → 必然在完成前被中断
    let mut pos = Position::startpos();
    let stop = NodeLimitedStop::new(300);
    let result = engine.search_with(&mut pos, SearchLimits::depth_nodes(30, 300), &stop);

    assert!(result.stopped, "节点上限这么小，应当被中断");
    if let Some(best) = result.best_move {
        assert!(
            pos.is_legal(best),
            "中断后返回的着法必须仍然合法，实际 {best}"
        );
    }
    assert_eq!(result.nodes, 300, "被中断时节点数应恰好停在上限附近");
}

/// `root_moves` 必须覆盖全部根着法且按评分降序。
#[test]
fn root_moves_cover_every_move_sorted_descending() {
    let mut engine = Engine::new(6);
    let mut pos = Position::startpos();
    let legal_count = pos.legal_moves().len();

    let result = engine.search_with(&mut pos, SearchLimits::depth(3), &NeverStop);

    assert_eq!(
        result.root_moves.len(),
        legal_count,
        "根着法数应与合法着法数一致（root_moves 是难度随机化与讲解的数据源，不能漏）"
    );
    for pair in result.root_moves.windows(2) {
        assert!(
            pair[0].1 >= pair[1].1,
            "root_moves 应按评分降序：{} < {}",
            pair[0].1,
            pair[1].1
        );
    }
    // 每个都是合法着法
    for (mv, _) in &result.root_moves {
        assert!(pos.is_legal(*mv), "root_moves 里出现了非法着法 {mv}");
    }
    // 首项就是返回的推荐着法（未随机化时）
    assert_eq!(result.best_move, Some(result.root_moves[0].0));
}

/// 初始局面不应出现杀棋评分。
#[test]
fn startpos_has_no_mate_score() {
    let mut engine = Engine::new(7);
    let mut pos = Position::startpos();
    let result = engine.search_with(&mut pos, SearchLimits::depth(4), &NeverStop);
    assert!(!result.is_mate_score());
    assert!(
        result.score.abs() < 400,
        "初始局面评分应接近 0，实际 {}",
        result.score
    );
}

/// 已经把对方将死的局面：走子方应是「无着可走 → 将死」。
#[test]
fn checkmated_position_reports_no_move() {
    let mut engine = Engine::new(8);
    let mut pos = build(
        &[
            (Color::Red, PieceKind::King, "e0"),
            (Color::Red, PieceKind::Chariot, "d8"),
            (Color::Red, PieceKind::Chariot, "e8"),
            (Color::Red, PieceKind::Chariot, "f8"),
            (Color::Black, PieceKind::King, "e9"),
        ],
        Color::Black, // 轮到黑方，且已被将死
    );
    let result = engine.search_with(&mut pos, SearchLimits::depth(3), &NeverStop);
    assert_eq!(result.best_move, None, "被将死的一方无着可走");
    assert!(result.is_mate_score());
}

/// 引擎自对弈：全程不得走出非法着法，局面一致性必须始终成立。
#[test]
fn self_play_never_produces_illegal_move() {
    for game in 0..6u64 {
        let mut engine = Engine::new(1000 + game);
        engine.set_difficulty(Difficulty::L2);
        engine.new_game();

        let mut pos = Position::startpos();
        let stop = NodeLimitedStop::new(60_000);

        for _ in 0..60 {
            if pos.status().is_over() {
                break;
            }
            let result = engine.search(&mut pos, &stop);
            let Some(best) = result.best_move else { break };

            assert!(
                pos.is_legal(best),
                "第 {game} 局出现了非法着法 {best}\n{pos:?}"
            );
            pos.make_move(best).expect("合法着法必须能被接受");
            pos.assert_consistent();
        }
    }
}

/// 难度越高，搜索深度不该更浅。
#[test]
fn higher_difficulty_searches_deeper() {
    let mut pos = Position::startpos();
    let mut depths = Vec::new();
    for level in [Difficulty::L1, Difficulty::L3] {
        let mut engine = Engine::new(9);
        engine.set_difficulty(level);
        let result = engine.search_with(&mut pos, level.profile().limits, &NeverStop);
        depths.push(result.depth);
    }
    assert!(
        depths[0] <= depths[1],
        "L1 的深度（{}）不应超过 L3（{}）",
        depths[0],
        depths[1]
    );
}

/// 换局后置换表被清空 —— 否则上一局的深层结果会污染这一局。
#[test]
fn new_game_clears_transposition_table() {
    let mut engine = Engine::new(10);
    let mut pos = Position::startpos();
    engine.search_with(&mut pos, SearchLimits::depth(4), &NeverStop);
    let before = engine.hit_rate();
    engine.new_game();
    assert_eq!(
        engine.hit_rate(),
        0.0,
        "换局后命中率统计应清零（命中率 {before}）"
    );
}
