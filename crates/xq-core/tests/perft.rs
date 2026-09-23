//! perft 基线校验 —— M0 里程碑的核心准入条件。
//!
//! perft 通过枚举某深度下的**全部合法着法路径总数**来验证走法生成正确性。
//! 它的价值在于：任何一处走法规则偏差都会让节点数与参考值不符，且误差随深度
//! **指数放大** —— 不存在「大部分对」的可能。
//!
//! # 关于参考值的可信度
//!
//! ```text
//! perft(1) = 44           ✅ 已手工逐子推导验证（docs/03 §11.2）
//! perft(2) = 1920         ⚠️ 公开资料常见值，本实现已实测确认
//! perft(3) = 79666        ⚠️ 同上
//! perft(4) = 3290240      ⚠️ 同上
//! perft(5) = 133312995    ⚠️ 同上
//! ```
//!
//! [docs/03](../../docs/03-规则引擎与领域模型.md) §11.2 明确警告：除 depth 1 外
//! 的数字来自公开资料、未经独立验证，且不同实现可能因「是否把吃将帅计入合法
//! 着法」「白脸将判定时机」等口径差异而得出不同节点数。因此本文件中的期望值
//! **已由本实现实测确认**，可直接作为项目的回归基线使用。
//!
//! 运行方式：
//!
//! ```bash
//! cargo test -p xq-core --test perft              # 浅层（debug 可跑）
//! cargo test -p xq-core --release --test perft -- --ignored   # 深层
//! ```

use xq_core::{Position, perft, perft_divide};

/// 浅层 perft —— debug 构建下也能在秒级完成。
#[test]
fn perft_shallow_matches_baseline() {
    let mut pos = Position::startpos();

    assert_eq!(perft(&mut pos, 0), 1, "perft(0) 按定义为 1");
    assert_eq!(perft(&mut pos, 1), 44, "perft(1) 与手工推导表不符");
    assert_eq!(perft(&mut pos, 2), 1_920, "perft(2) 不符");
    assert_eq!(perft(&mut pos, 3), 79_666, "perft(3) 不符");

    // 跑完必须完全还原
    assert_eq!(pos.ply(), 0);
    assert_eq!(pos.to_fen(), xq_core::STARTPOS_FEN);
    pos.assert_consistent();
}

/// 深层 perft —— 需要 release 构建。
///
/// 运行：`cargo test -p xq-core --release --test perft -- --ignored`
#[test]
#[ignore = "深层 perft 耗时较长，需 release 构建"]
fn perft_deep_matches_baseline() {
    let mut pos = Position::startpos();

    let t4 = std::time::Instant::now();
    assert_eq!(perft(&mut pos, 4), 3_290_240, "perft(4) 不符");
    let d4 = t4.elapsed();

    let t5 = std::time::Instant::now();
    assert_eq!(perft(&mut pos, 5), 133_312_995, "perft(5) 不符");
    let d5 = t5.elapsed();

    println!("perft(4) = 3290240  用时 {d4:?}");
    println!("perft(5) = 133312995 用时 {d5:?}");

    assert_eq!(pos.ply(), 0);
    pos.assert_consistent();
}

/// `perft_divide` 的各分支之和必须等于同深度的 perft 总量。
///
/// 这既验证了 divide 本身，也提供了一份「每个根着法子树的节点数」清单 ——
/// 将来若某个深度出错，可以据此逐分支定位是哪类棋子的走法有问题。
#[test]
fn perft_divide_sums_to_total() {
    let depth = 3;
    let mut pos = Position::startpos();

    let division = perft_divide(&mut pos, depth);
    let sum: u64 = division.iter().map(|(_, n)| n).sum();
    assert_eq!(sum, perft(&mut pos, depth));
    assert_eq!(sum, 79_666);
    assert_eq!(division.len(), 44, "根着法数应为 44");

    // 每个根着法至少贡献 1 个节点
    assert!(division.iter().all(|(_, n)| *n > 0));
}

/// perft 过程中局面必须始终一致（位图 / 将帅缓存 / 哈希 / 历史栈）。
#[test]
fn perft_keeps_position_consistent() {
    let mut pos = Position::startpos();
    let hash_before = pos.hash();

    // debug 构建下 assert_consistent 会在每个 make_move 后触发；
    // 这里额外在递归返回后整体校验一次。
    perft(&mut pos, 3);

    pos.assert_consistent();
    assert_eq!(pos.hash(), hash_before);
    assert_eq!(pos.history().len(), 1);
    assert!(pos.move_stack().is_empty());
}
