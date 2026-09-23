//! 属性测试与自对弈压力测试。
//!
//! 这些是覆盖面最广的回归手段：随机对局能走到人工很难构造的局面组合，
//! 一旦某条规则分支写错，几乎必然在某次随机游走中暴露。
//!
//! 随机源是**自带种子的确定性 PRNG**（见 `common/mod.rs`），因此失败可精确复现。

mod common;

use common::TestRng;
use proptest::prelude::*;
use xq_core::{Color, Position, from_iccs};

/// 从初始局面随机走 `plies` 步，返回最终局面与走过的着法序列。
fn random_playout(seed: u64, plies: usize) -> (Position, Vec<xq_core::Move>) {
    let mut rng = TestRng::new(seed);
    let mut pos = Position::startpos();
    let mut played = Vec::with_capacity(plies.max(1));

    for _ in 0..plies {
        let legal = pos.legal_moves();
        if legal.is_empty() {
            break;
        }
        let mv = legal[rng.below(legal.len())];
        pos.make_move(mv).expect("合法着法必须能被 make_move 接受");
        played.push(mv);
    }
    (pos, played)
}

proptest! {
    /// 随机局面的 FEN 往返必须无损。
    ///
    /// 检查三层：FEN 字符串完全一致、Zobrist 哈希一致、90 格棋盘逐字节一致。
    #[test]
    fn fen_roundtrip_is_lossless(seed in any::<u64>(), plies in 1usize..=120) {
        let (pos, _) = random_playout(seed, plies);

        let fen = pos.to_fen();
        let back = Position::from_fen(&fen)
            .unwrap_or_else(|e| panic!("自家生成的 FEN 竟然无法解析: {e}\nFEN = {fen}"));

        prop_assert_eq!(back.to_fen(), fen, "FEN 二次序列化不一致");
        prop_assert_eq!(back.hash(), pos.hash(), "FEN 往返后哈希不一致");
        prop_assert_eq!(back.squares(), pos.squares(), "FEN 往返后棋盘不一致");
        prop_assert_eq!(back.side_to_move(), pos.side_to_move());
        prop_assert_eq!(back.halfmove_clock(), pos.halfmove_clock());
        prop_assert_eq!(back.fullmove_number(), pos.fullmove_number());
    }

    /// 随机对局的每一步：中文记谱往返必须还原原着法。
    #[test]
    fn chinese_notation_roundtrip_is_lossless(seed in any::<u64>(), plies in 1usize..=120) {
        let mut rng = TestRng::new(seed);
        let mut pos = Position::startpos();

        for _ in 0..plies {
            let legal = pos.legal_moves();
            if legal.is_empty() {
                break;
            }
            let mv = legal[rng.below(legal.len())];

            let text = pos
                .to_chinese_notation(mv)
                .unwrap_or_else(|e| panic!("着法 {mv} 生成记谱失败: {e}"));
            let parsed = pos
                .from_chinese_notation(&text)
                .unwrap_or_else(|e| panic!("自家生成的记谱 {text} 竟然无法解析: {e}"));

            prop_assert_eq!(parsed, mv, "记谱 {} 往返后着法不一致", text);
            pos.make_move(mv).unwrap();
        }
    }

    /// make / unmake 必须严格互逆：随意走一段再全部撤销，应逐字节回到初始局面。
    #[test]
    fn make_unmake_is_symmetric(seed in any::<u64>(), plies in 1usize..=150) {
        let (mut pos, played) = random_playout(seed, plies);
        let expected_fen = pos.to_fen();
        let expected_hash = pos.hash();

        // 全部撤销
        for _ in 0..played.len() {
            pos.unmake_move().expect("unmake 栈不应下溢");
        }

        prop_assert_eq!(pos.ply(), 0);
        prop_assert_eq!(pos.to_fen(), xq_core::STARTPOS_FEN, "全部撤销后未回到初始局面");
        prop_assert_eq!(pos.hash(), Position::startpos().hash());
        prop_assert_eq!(pos.history().len(), 1);

        // 再重放一遍，应当得到完全相同的局面
        for mv in &played {
            pos.make_move(*mv).unwrap();
        }
        prop_assert_eq!(pos.to_fen(), expected_fen, "重放后局面不一致");
        prop_assert_eq!(pos.hash(), expected_hash, "重放后哈希不一致");
    }

    /// 随机对局全程不得出现非法状态：哈希自洽、将帅各一、走子方合法、着法数有限。
    #[test]
    fn random_playout_never_reaches_illegal_state(seed in any::<u64>(), plies in 1usize..=150) {
        let mut rng = TestRng::new(seed);
        let mut pos = Position::startpos();

        for step in 0..plies {
            // 每步都校验内部一致性（debug 构建下 assert_consistent 有效）
            pos.assert_consistent();

            prop_assert!(pos.king_in_palace(Color::Red), "第 {step} 步红帅跑出九宫");
            prop_assert!(pos.king_in_palace(Color::Black), "第 {step} 步黑将跑出九宫");

            let legal = pos.legal_moves();
            prop_assert!(legal.len() <= xq_core::MAX_MOVES, "着法数超出上限");

            if legal.is_empty() {
                // 无着可走 → 必须是终局
                let status = pos.status();
                prop_assert!(status.is_over(), "无合法着法却未判定终局: {status:?}");
                break;
            }

            let mv = legal[rng.below(legal.len())];
            // 走子方必须拥有起点上的棋子
            prop_assert_eq!(
                xq_core::color_of(pos.piece_at(mv.from())),
                Some(pos.side_to_move()),
                "着法起点不是走子方的棋子"
            );
            pos.make_move(mv).unwrap();
        }
    }

    /// ICCS 坐标往返：任意合法着法的起止格都必须是合法索引。
    #[test]
    fn all_legal_moves_have_valid_squares(seed in any::<u64>(), plies in 1usize..=60) {
        let (mut pos, _) = random_playout(seed, plies);
        for mv in pos.legal_moves() {
            let from = mv.from();
            let to = mv.to();
            prop_assert!(xq_core::is_valid_index(from), "起点越界: {from}");
            prop_assert!(xq_core::is_valid_index(to), "终点越界: {to}");
            prop_assert_ne!(from, to, "起点与终点相同");

            // ICCS 字符串往返
            let text = pos.to_iccs_string(mv);
            prop_assert_eq!(text.len(), 4);
            prop_assert_eq!(from_iccs(&text[0..2]), Some(from));
            prop_assert_eq!(from_iccs(&text[2..4]), Some(to));
        }
    }
}

/// 随机自对弈：全程不得 panic、不得走出非法着法。
///
/// 这是覆盖面最广的回归手段 —— 一次随机对局会触及几十个不同局面，
/// 规则里任何一条分支写错都极难在 100 局里完全躲过。
#[test]
fn random_self_play_smoke() {
    for game in 0..100u64 {
        let mut rng = TestRng::new(game ^ 0xA5A5_5A5A);
        let mut pos = Position::startpos();
        let mut plies = 0usize;

        loop {
            let legal = pos.legal_moves();
            if legal.is_empty() || plies >= 300 {
                break;
            }
            let mv = legal[rng.below(legal.len())];
            pos.make_move(mv).expect("随机选出的合法着法竟被拒绝");
            plies += 1;
        }

        pos.assert_consistent();
        assert_eq!(pos.ply(), plies);
    }
}

/// 1000 局自对弈压力测试（release 构建）。
///
/// 运行：`cargo test -p xq-core --release --test properties -- --ignored`
#[test]
#[ignore = "1000 局自对弈较慢，需 release 构建"]
fn random_self_play_stress_1000_games() {
    let mut total_plies = 0u64;
    let mut finished_games = 0u64;

    for game in 0..1000u64 {
        let mut rng = TestRng::new(game);
        let mut pos = Position::startpos();

        loop {
            let legal = pos.legal_moves();
            if legal.is_empty() || pos.ply() >= 400 {
                break;
            }
            // 优先选吃子着法会让局面更激烈，但不影响正确性检查
            let mv = legal[rng.below(legal.len())];
            pos.make_move(mv).expect("合法着法被拒绝");
        }

        if pos.status().is_over() {
            finished_games += 1;
        }
        total_plies += pos.ply() as u64;
        pos.assert_consistent();
    }

    println!("1000 局自对弈完成：总步数 {total_plies}，自然终局 {finished_games} 局");
    assert!(total_plies > 0);
}
