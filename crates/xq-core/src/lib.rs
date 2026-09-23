//! # xq-core —— 中国象棋规则内核
//!
//! 走法生成、局面表示、终局判定、FEN 与中文记谱。**零 IO、零运行时依赖**。
//!
//! 本 crate 是整个项目的正确性基石：客户端与服务端复用同一份实现，物理上
//! 只有一套规则（见 [docs/02](../../../docs/02-系统架构设计.md) §2.1）。
//!
//! ## 设计约束
//!
//! 遵守不变量 **I-1（领域层零 IO）**：不依赖 `tokio` / `axum` / `sqlx` /
//! `sea-orm` / `redis` / `reqwest` / `hyper`，也不使用 `std::fs`、`std::net`、
//! `std::time::SystemTime`。**连 `rand` 都不引入** —— Zobrist 用内置的
//! SplitMix64 生成，这样「不引入系统随机」由代码结构保证而非靠约定。
//!
//! ## 坐标约定
//!
//! 内部统一使用 **ICCS 坐标**：列 `a`..`i` = 0..8，行 `0`..`9`，
//! **`row 0` 是红方底线**。详见 [`square`] 模块文档。
//!
//! ## 快速上手
//!
//! ```
//! use xq_core::{Position, perft};
//!
//! // 标准初始局面
//! let mut pos = Position::startpos();
//!
//! // 走一步「炮二平五」
//! let mv = pos.from_chinese_notation("炮二平五").expect("记谱应可解析");
//! pos.make_move(mv).expect("着法应合法");
//!
//! // 黑方有 45 种应法 —— 注意比初始的 44 多 1 步。
//! // 原因：走的是 h2 那个炮，h 列让开后，黑方 h7 炮向下的通路
//! // 从 4 格延长到 6 格。b2 的红炮原地未动，故 b7 炮仍是 12 步。
//! let replies = pos.legal_moves();
//! assert_eq!(replies.len(), 45);
//!
//! // perft 验证走法生成正确性
//! let mut fresh = Position::startpos();
//! assert_eq!(perft(&mut fresh, 1), 44);
//!
//! // 局面可无损往返 FEN
//! let fen = fresh.to_fen();
//! assert_eq!(Position::from_fen(&fen).unwrap().to_fen(), fen);
//! ```
//!
//! ## 两套走法生成 API
//!
//! | API | 分配 | 用途 |
//! |---|---|---|
//! | [`Position::legal_moves`] | 返回 `Vec` | UI 高亮（每帧至多几次调用） |
//! | [`Position::gen_legal_into`] | 写入调用方的 [`MoveList`] | 搜索内层循环 |
//! | [`Position::has_legal_move`] | 无分配、短路 | 终局判定与搜索 |
//!
//! 这是**刻意的分工**：UI 要方便，搜索要零分配。

#![deny(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod color;
pub mod fen;
pub mod movegen;
pub mod mv;
pub mod nature;
pub mod notation;
pub mod piece;
pub mod position;
pub mod repetition;
pub mod rules;
pub mod square;
pub mod zobrist;

pub use color::Color;
pub use fen::{FenError, STARTPOS_FEN, from_fen, to_fen};
pub use mv::{MAX_MOVES, Move, MoveList, MoveRecord};
pub use nature::MoveNature;
pub use notation::{Action, MoveDesc, NotationError, Subject};
pub use piece::{EMPTY, PieceKind, color_of, encode, is_empty, kind_of};
pub use position::{IllegalMove, Position, PositionError, position_from_pieces};
pub use repetition::{RepetitionVerdict, adjudicate, is_threefold, occurrences};
pub use rules::{DrawReason, GameStatus, SIXTY_MOVE_HALF_MOVES, perft, perft_divide};
pub use square::{
    BOARD_SIZE, COLS, ROWS, col_of, from_iccs, index, is_valid_index, on_board, route_number,
    row_of, to_iccs,
};

/// 常用类型的集中导入。
///
/// ```
/// use xq_core::prelude::*;
/// let pos = Position::startpos();
/// assert_eq!(pos.side_to_move(), Color::Red);
/// ```
pub mod prelude {
    pub use crate::color::Color;
    pub use crate::mv::{Move, MoveList, MoveRecord};
    pub use crate::nature::MoveNature;
    pub use crate::piece::PieceKind;
    pub use crate::position::Position;
    pub use crate::rules::{DrawReason, GameStatus, perft};
    pub use crate::square::{from_iccs, index, to_iccs};
}
