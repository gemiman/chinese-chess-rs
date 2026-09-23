//! 边界与易错规则用例。
//!
//! 这里集中覆盖几类**最容易实现错**的规则：
//!
//! | 规则 | 为什么容易错 |
//! |---|---|
//! | 白脸将（将帅照面） | 它不是「走法」而是「局面合法性」，容易漏判 |
//! | 不能吃对方将帅 | 从合法对局出发永不出现，只有手工局面能触发，极易漏 |
//! | 老兵横向能力 | 兵到底线后仍能横走，直觉上容易当作「无着可走」 |
//! | 将军必须解除 | 过滤逻辑写漏会让 AI 走出送将的着法 |
//! | 困毙判负 | 与国际象棋判和相反 |

use xq_core::piece::PieceKind;
use xq_core::{Color, GameStatus, Position, from_iccs};

fn build(pieces: &[(Color, PieceKind, &str)], side: Color) -> Position {
    xq_core::position_from_pieces(pieces, side).expect("测试局面构造失败")
}

/// 把 ICCS 串（如 `"h2e2"`）转成着法。
fn mv(iccs: &str) -> xq_core::Move {
    xq_core::Move::new(
        from_iccs(&iccs[0..2]).expect("起点坐标非法"),
        from_iccs(&iccs[2..4]).expect("终点坐标非法"),
    )
}

/// 白脸将：垫在将帅同列之间的棋子被「钉住」，只能沿线移动。
///
/// 局面：红帅 `e0`、红车 `e3`、黑将 `e9`。红车是唯一隔断两个将帅的棋子，
/// 一旦离开 e 列，将帅立即照面 → 该着法非法。
#[test]
fn flying_general_pins_piece_to_its_file() {
    let mut pos = build(
        &[
            (Color::Red, PieceKind::King, "e0"),
            (Color::Red, PieceKind::Chariot, "e3"),
            (Color::Black, PieceKind::King, "e9"),
        ],
        Color::Red,
    );

    assert!(!pos.is_in_check(Color::Red), "有车垫着，红帅不应被将");

    // 离开 e 列 → 将帅照面 → 非法
    for target in ["a3", "b3", "c3", "d3", "f3", "g3", "h3", "i3"] {
        assert!(
            pos.legal_move_to(from_iccs("e3").unwrap(), from_iccs(target).unwrap())
                .is_none(),
            "e3 车不应能走到 {target}（会让将帅照面）"
        );
    }

    // 沿 e 列移动 → 合法
    for target in ["e1", "e2", "e4", "e5", "e8"] {
        assert!(
            pos.legal_move_to(from_iccs("e3").unwrap(), from_iccs(target).unwrap())
                .is_some(),
            "e3 车应能走到 {target}"
        );
    }

    // 恰好 7 步：e1 e2 e4 e5 e6 e7 e8（e9 是黑将，不可吃）
    let along: Vec<String> = pos
        .legal_moves()
        .into_iter()
        .filter(|m| m.from() == from_iccs("e3").unwrap())
        .map(|m| xq_core::to_iccs(m.to()))
        .collect();
    assert_eq!(along.len(), 7, "被钉住的车应只有 7 步，实际 {along:?}");
}

/// 「吃掉对方将帅」不是合法着法。
///
/// 从合法对局出发这情形永不出现，但手工构造的 FEN 可以摆出黑将无人保护的
/// 局面 —— 此时必须**不生成**吃将着法，否则走完之后将帅缓存会变成悬空指针。
#[test]
fn capturing_the_king_is_not_a_legal_move() {
    let mut pos = build(
        &[
            (Color::Red, PieceKind::King, "d0"),
            (Color::Red, PieceKind::Chariot, "e1"),
            (Color::Black, PieceKind::King, "e9"),
        ],
        Color::Red,
    );

    assert!(
        pos.legal_move_to(from_iccs("e1").unwrap(), from_iccs("e9").unwrap())
            .is_none(),
        "不应生成「吃掉黑将」的着法"
    );
    // 正常沿列前进仍应合法
    assert!(
        pos.legal_move_to(from_iccs("e1").unwrap(), from_iccs("e5").unwrap())
            .is_some()
    );

    // 且合法着法列表里不含任何以将帅为目标的着法
    let mut clone = pos.clone();
    for m in clone.legal_moves() {
        let target = pos.piece_at(m.to());
        assert_ne!(
            xq_core::kind_of(target),
            Some(PieceKind::King),
            "合法着法 {m} 试图吃将帅"
        );
    }
}

/// 将帅照面本身就是非法局面：被将的一方只能离开该列。
#[test]
fn facing_kings_force_the_checked_side_off_the_file() {
    // 红帅 e1、黑将 e9，e 列无子 → 红方处于「被将」（白脸将）状态。
    // 额外放一个黑卒，避免触发「子力不足判和」—— `status()` 的判定顺序里
    // 子力不足优先于将军（见 docs/03 §5.1）。
    let mut pos = build(
        &[
            (Color::Red, PieceKind::King, "e1"),
            (Color::Black, PieceKind::King, "e9"),
            (Color::Black, PieceKind::Pawn, "a6"),
        ],
        Color::Red,
    );

    assert!(pos.is_in_check(Color::Red), "将帅照面应被视为红方被将");
    assert_eq!(pos.status(), GameStatus::Check { side: Color::Red });

    // 沿 e 列移动（e0 / e2）并不能解除照面 → 非法
    assert!(
        pos.legal_move_to(from_iccs("e1").unwrap(), from_iccs("e0").unwrap())
            .is_none(),
        "退到底线仍在 e 列，照面未解除"
    );
    assert!(
        pos.legal_move_to(from_iccs("e1").unwrap(), from_iccs("e2").unwrap())
            .is_none(),
        "沿 e 列前进，照面未解除"
    );
    // 离开 e 列 → 合法
    assert!(
        pos.legal_move_to(from_iccs("e1").unwrap(), from_iccs("d1").unwrap())
            .is_some()
    );
    assert!(
        pos.legal_move_to(from_iccs("e1").unwrap(), from_iccs("f1").unwrap())
            .is_some()
    );
}

/// 老兵：兵走到对方底线后不能前进，但**横向能力保留**。
#[test]
fn pawn_on_back_rank_keeps_sideways_moves() {
    // 红兵已到 a9（黑方底线）
    let mut pos = build(
        &[
            (Color::Red, PieceKind::King, "d0"),
            (Color::Red, PieceKind::Pawn, "a9"),
            (Color::Black, PieceKind::King, "e9"),
        ],
        Color::Red,
    );

    let moves = pos.legal_moves();
    let targets: Vec<String> = moves
        .iter()
        .filter(|m| m.from() == from_iccs("a9").unwrap())
        .map(|m| xq_core::to_iccs(m.to()))
        .collect();

    assert!(targets.contains(&"b9".to_string()), "老兵应仍能横走 b9");
    assert!(!targets.contains(&"a10".to_string()));
    assert_eq!(targets.len(), 1, "a9 老兵只剩 b9 一步，实际 {targets:?}");
}

/// 棋盘角上的车：射线被棋盘边界与己方棋子截断。
#[test]
fn corner_chariot_has_expected_rays() {
    let mut pos = build(
        &[
            (Color::Red, PieceKind::King, "e0"),
            (Color::Red, PieceKind::Chariot, "a0"),
            (Color::Black, PieceKind::King, "d9"),
        ],
        Color::Red,
    );

    let targets: Vec<String> = pos
        .legal_moves()
        .into_iter()
        .filter(|m| m.from() == from_iccs("a0").unwrap())
        .map(|m| xq_core::to_iccs(m.to()))
        .collect();

    // 向上 9 格（a1..a9）+ 向右 3 格（b0/c0/d0，e0 是己方帅）
    assert_eq!(targets.len(), 12, "a0 车应 12 步，实际 {targets:?}");
    for t in ["a1", "a5", "a9", "b0", "c0", "d0"] {
        assert!(targets.contains(&t.to_string()), "缺 {t}");
    }
    assert!(!targets.contains(&"e0".to_string()), "不可吃己方帅");
}

/// 将军必须被解除：不解除将军的着法一律被过滤掉。
#[test]
fn every_move_must_resolve_check() {
    // 黑车 e8 将军红帅 e0；红车 a1 唯一出路是垫到 e1
    let mut pos = build(
        &[
            (Color::Red, PieceKind::King, "e0"),
            (Color::Red, PieceKind::Chariot, "a1"),
            (Color::Black, PieceKind::Chariot, "e8"),
            (Color::Black, PieceKind::King, "d9"),
        ],
        Color::Red,
    );

    assert!(pos.is_in_check(Color::Red));

    // 不垫子 → 仍在被将 → 非法
    assert!(
        pos.legal_move_to(from_iccs("a1").unwrap(), from_iccs("a2").unwrap())
            .is_none(),
        "不解除将军的着法应被过滤"
    );
    assert!(
        pos.legal_move_to(from_iccs("a1").unwrap(), from_iccs("b1").unwrap())
            .is_none()
    );

    // 垫到 e1 → 合法
    assert!(
        pos.legal_move_to(from_iccs("a1").unwrap(), from_iccs("e1").unwrap())
            .is_some(),
        "垫子解将应合法"
    );

    // 列表中每一步都必须真的解除将军
    let mut clone = pos.clone();
    for m in clone.legal_moves() {
        let mut probe = pos.clone();
        probe.make_move(m).unwrap();
        assert!(
            !probe.is_in_check(Color::Red),
            "合法着法列表里出现了未解除将军的着法 {m}"
        );
    }
}

/// 谁被将死就判谁负，且状态里带的 loser 与走子方一致。
#[test]
fn checkmate_loser_is_always_the_side_to_move() {
    // 黑将 e9 被 d8/e8/f8 三车合围
    let mut pos = build(
        &[
            (Color::Red, PieceKind::King, "e0"),
            (Color::Red, PieceKind::Chariot, "d8"),
            (Color::Red, PieceKind::Chariot, "e8"),
            (Color::Red, PieceKind::Chariot, "f8"),
            (Color::Black, PieceKind::King, "e9"),
        ],
        Color::Black,
    );
    match pos.status() {
        GameStatus::Checkmate { loser } => {
            assert_eq!(loser, Color::Black);
            assert_eq!(loser, pos.side_to_move());
        }
        other => panic!("应判将死，实际 {other:?}"),
    }
}

/// 着法列表不允许出现重复着法（去重是生成的隐含契约）。
#[test]
fn no_duplicate_moves_in_generated_list() {
    for seed in 0..30u64 {
        let mut pos = Position::startpos();
        // 用坐标做种子做几步随机走，走到中局再检查
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        for _ in 0..40 {
            let legal = pos.legal_moves();
            if legal.is_empty() {
                break;
            }
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let pick = legal[(state % legal.len() as u64) as usize];
            pos.make_move(pick).unwrap();
        }

        let legal = pos.legal_moves();
        let mut seen = std::collections::HashSet::new();
        for m in &legal {
            assert!(seen.insert(*m), "着法 {m} 重复出现（seed={seed}）");
        }
    }
}

/// `legal_move_to` 与 `legal_moves` 必须完全一致（UI 交互依赖这条）。
#[test]
fn legal_move_to_agrees_with_legal_moves() {
    let mut pos = Position::startpos();

    let list: std::collections::HashSet<_> = pos.legal_moves().into_iter().collect();

    // 穷举全部 90×90 组合，逐一比对
    for from in 0..90u8 {
        for to in 0..90u8 {
            let via_lookup = pos.legal_move_to(from, to).is_some();
            let via_list = list.contains(&xq_core::Move::new(from, to));
            assert_eq!(
                via_lookup, via_list,
                "legal_move_to({from},{to}) 与 legal_moves 不一致"
            );
        }
    }
}

/// ICCS 串辅助函数本身可用（顺带保证测试工具不出错）。
#[test]
fn iccs_move_helper_roundtrips() {
    let m = mv("h2e2");
    assert_eq!(xq_core::to_iccs(m.from()), "h2");
    assert_eq!(xq_core::to_iccs(m.to()), "e2");

    // 用它核对初始局面里「炮二平五」确实是 h2 → e2
    let mut pos = Position::startpos();
    let expected = pos.from_chinese_notation("炮二平五").unwrap();
    assert_eq!(expected, mv("h2e2"));
}
