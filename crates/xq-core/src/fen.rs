//! FEN 解析与序列化。
//!
//! # 格式
//!
//! ```text
//! <棋盘> <走子方> <保留1> <保留2> <半步计数> <回合数>
//! ```
//!
//! | 字段 | 说明 |
//! |---|---|
//! | 棋盘 | 从 `row 9` 到 `row 0`，10 段以 `/` 分隔；每段内用数字表示连续空格数 |
//! | 走子方 | `w` = 红方，`b` = 黑方 |
//! | 保留1 / 保留2 | 中国象棋无易位与吃过路兵，固定为 `-` |
//! | 半步计数 | 距上次吃子的半步数（用于 60 回合规则） |
//! | 回合数 | 从 1 开始 |
//!
//! 棋子字母与彩色 `xq-client` 的棋盘渲染共用同一套约定：
//! 大写为红（`K A B N R C P`），小写为黑（`k a b n r c p`）。
//!
//! > ⚠️ **段序方向极易写反**：FEN 的第一段对应 `row 9`（黑方底线），
//! > 最后一段对应 `row 0`（红方底线）—— 与棋理上「红方在下方」的视觉顺序相反。

use crate::color::Color;
use crate::piece::{EMPTY, PieceKind, color_of, encode, kind_of};
use crate::position::{Position, PositionError};
use crate::square::{BOARD_SIZE, COLS, ROWS, index};

/// 标准初始局面 FEN（唯一正确形式）。
pub const STARTPOS_FEN: &str =
    "rnbakabnr/9/1c5c1/p1p1p1p1p/9/9/P1P1P1P1P/1C5C1/9/RNBAKABNR w - - 0 1";

/// FEN 解析错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenError {
    /// 字段数不足（至少需要「棋盘 + 走子方」两段）。
    TooFewFields(usize),
    /// 棋盘段数不是 10。
    RowCount(usize),
    /// 某行宽度不足或超出 9。
    RowWidth { row: u8, width: usize },
    /// 出现了无法识别的字符。
    BadChar { ch: char, row: u8 },
    /// 连续空格数字超出 1..=9。
    BadGapDigit { ch: char, row: u8 },
    /// 走子方字段无法识别。
    BadSideToMove(String),
    /// 元信息字段（半步计数 / 回合数）不是合法整数。
    BadNumber(String),
    /// 局面本身非法（缺将帅 / 多个将帅）。
    BadPosition(PositionError),
    /// 将帅不在己方九宫内。
    KingOutsidePalace(Color),
}

impl core::fmt::Display for FenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FenError::TooFewFields(n) => write!(f, "FEN 字段数不足（{n} 个，至少需要 2 个）"),
            FenError::RowCount(n) => write!(f, "FEN 棋盘段数应为 10，实际 {n}"),
            FenError::RowWidth { row, width } => {
                write!(f, "FEN 第 {row} 行宽度应为 9，实际 {width}")
            }
            FenError::BadChar { ch, row } => write!(f, "FEN 第 {row} 行出现非法字符 '{ch}'"),
            FenError::BadGapDigit { ch, row } => {
                write!(f, "FEN 第 {row} 行的空格数字 '{ch}' 非法（应为 1..=9）")
            }
            FenError::BadSideToMove(s) => write!(f, "FEN 走子方字段非法: '{s}'"),
            FenError::BadNumber(s) => write!(f, "FEN 元信息字段不是合法整数: '{s}'"),
            FenError::BadPosition(e) => write!(f, "FEN 描述的局面非法: {e}"),
            FenError::KingOutsidePalace(c) => write!(f, "{}方将帅不在九宫内", c.name_zh()),
        }
    }
}

impl std::error::Error for FenError {}

impl From<PositionError> for FenError {
    fn from(e: PositionError) -> Self {
        FenError::BadPosition(e)
    }
}

/// 序列化为 FEN。
pub fn to_fen(pos: &Position) -> String {
    let mut s = String::with_capacity(96);

    // 从 row 9（黑方底线）到 row 0（红方底线）
    for i in 0..ROWS {
        let row = ROWS - 1 - i;
        let mut gap = 0u8;
        for col in 0..COLS {
            let piece = pos.piece_at(index(col, row));
            if piece == EMPTY {
                gap += 1;
                continue;
            }
            if gap > 0 {
                s.push((b'0' + gap) as char);
                gap = 0;
            }
            let kind = kind_of(piece).expect("非空棋子必有种类");
            let color = color_of(piece).expect("非空棋子必有颜色");
            s.push(kind.fen_char(color));
        }
        if gap > 0 {
            s.push((b'0' + gap) as char);
        }
        if i + 1 < ROWS {
            s.push('/');
        }
    }

    s.push(' ');
    s.push(pos.side_to_move().fen_char());
    s.push_str(" - - ");
    s.push_str(&pos.halfmove_clock().to_string());
    s.push(' ');
    s.push_str(&pos.fullmove_number().to_string());
    s
}

/// 从 FEN 解析。
///
/// 接受两种形式：
/// - 完整 6 段（标准形式）；
/// - 仅 2 段（棋盘 + 走子方），此时半步计数取 0、回合数取 1。
pub fn from_fen(fen: &str) -> Result<Position, FenError> {
    let fields: Vec<&str> = fen.split_whitespace().collect();
    if fields.len() < 2 {
        return Err(FenError::TooFewFields(fields.len()));
    }

    let board = fields[0];
    let side_str = fields[1];
    let side = Color::from_fen_char(
        side_str
            .chars()
            .next()
            .ok_or_else(|| FenError::BadSideToMove(side_str.to_string()))?,
    )
    .ok_or_else(|| FenError::BadSideToMove(side_str.to_string()))?;

    let halfmove_clock: u16 = if fields.len() > 4 {
        fields[4]
            .parse()
            .map_err(|_| FenError::BadNumber(fields[4].to_string()))?
    } else {
        0
    };
    let fullmove_number: u16 = if fields.len() > 5 {
        fields[5]
            .parse()
            .map_err(|_| FenError::BadNumber(fields[5].to_string()))?
    } else {
        1
    };

    let rows: Vec<&str> = board.split('/').collect();
    if rows.len() != ROWS as usize {
        return Err(FenError::RowCount(rows.len()));
    }

    let mut squares = [EMPTY; BOARD_SIZE];
    for (i, segment) in rows.iter().enumerate() {
        // 第 0 段是 row 9，最后一段是 row 0
        let row = ROWS - 1 - i as u8;
        let mut col = 0u8;

        for ch in segment.chars() {
            if let Some(d) = ch.to_digit(10) {
                if d == 0 || d > 9 {
                    return Err(FenError::BadGapDigit { ch, row });
                }
                col = col.saturating_add(d as u8);
                if col > COLS {
                    return Err(FenError::RowWidth {
                        row,
                        width: col as usize,
                    });
                }
                continue;
            }

            let (kind, color) =
                PieceKind::from_fen_char(ch).ok_or(FenError::BadChar { ch, row })?;
            if col >= COLS {
                return Err(FenError::RowWidth {
                    row,
                    width: col as usize + 1,
                });
            }
            squares[index(col, row) as usize] = encode(color, kind);
            col += 1;
        }

        if col != COLS {
            return Err(FenError::RowWidth {
                row,
                width: col as usize,
            });
        }
    }

    let pos = Position::from_squares(squares, side, halfmove_clock, fullmove_number)?;

    // 将帅必须各在己方九宫内 —— 这是最容易被手写 FEN 违反的一条。
    for color in [Color::Red, Color::Black] {
        if !pos.king_in_palace(color) {
            return Err(FenError::KingOutsidePalace(color));
        }
    }

    Ok(pos)
}

impl Position {
    /// 从 FEN 构造。
    pub fn from_fen(fen: &str) -> Result<Position, FenError> {
        from_fen(fen)
    }

    /// 序列化为 FEN。
    pub fn to_fen(&self) -> String {
        to_fen(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startpos_fen_roundtrip() {
        let pos = from_fen(STARTPOS_FEN).expect("内置初始 FEN 必须可解析");
        assert_eq!(to_fen(&pos), STARTPOS_FEN);
    }

    /// 逐段校验初始局面（设计文档 §8.3 的表）。
    #[test]
    fn startpos_layout_matches_doc() {
        let pos = from_fen(STARTPOS_FEN).unwrap();

        // 黑方底线 row 9：车马象士将士象马车
        let expect_black = "rnbakabnr";
        let actual: String = (0..COLS)
            .map(|col| {
                let p = pos.piece_at(index(col, 9));
                kind_of(p).unwrap().fen_char(color_of(p).unwrap())
            })
            .collect();
        assert_eq!(actual, expect_black);

        // 红方底线 row 0：RNBAKABNR
        let actual: String = (0..COLS)
            .map(|col| {
                let p = pos.piece_at(index(col, 0));
                kind_of(p).unwrap().fen_char(color_of(p).unwrap())
            })
            .collect();
        assert_eq!(actual, "RNBAKABNR");

        // 黑炮在 b7 与 h7
        assert_eq!(kind_of(pos.piece_at(index(1, 7))), Some(PieceKind::Cannon));
        assert_eq!(color_of(pos.piece_at(index(1, 7))), Some(Color::Black));
        assert_eq!(kind_of(pos.piece_at(index(7, 7))), Some(PieceKind::Cannon));

        // 黑卒在 a6 c6 e6 g6 i6
        for col in [0u8, 2, 4, 6, 8] {
            assert_eq!(kind_of(pos.piece_at(index(col, 6))), Some(PieceKind::Pawn));
            assert_eq!(color_of(pos.piece_at(index(col, 6))), Some(Color::Black));
        }

        // 红兵在 a3 c3 e3 g3 i3
        for col in [0u8, 2, 4, 6, 8] {
            assert_eq!(kind_of(pos.piece_at(index(col, 3))), Some(PieceKind::Pawn));
            assert_eq!(color_of(pos.piece_at(index(col, 3))), Some(Color::Red));
        }

        // 红炮在 b2 与 h2
        assert_eq!(kind_of(pos.piece_at(index(1, 2))), Some(PieceKind::Cannon));
        assert_eq!(color_of(pos.piece_at(index(1, 2))), Some(Color::Red));
        assert_eq!(kind_of(pos.piece_at(index(7, 2))), Some(PieceKind::Cannon));

        // 走子方为红
        assert_eq!(pos.side_to_move(), Color::Red);
    }

    /// 只给两段也应能解析，元信息取默认值。
    #[test]
    fn two_field_form_is_accepted() {
        let pos =
            from_fen("rnbakabnr/9/1c5c1/p1p1p1p1p/9/9/P1P1P1P1P/1C5C1/9/RNBAKABNR b").unwrap();
        assert_eq!(pos.side_to_move(), Color::Black);
        assert_eq!(pos.halfmove_clock(), 0);
        assert_eq!(pos.fullmove_number(), 1);
    }

    #[test]
    fn malformed_fens_are_rejected() {
        assert!(matches!(
            from_fen("rnbakabnr"),
            Err(FenError::TooFewFields(1))
        ));
        assert!(matches!(
            from_fen("9/9/9/9/9/9/9/9/9 w - - 0 1"),
            Err(FenError::RowCount(9))
        ));
        assert!(matches!(
            from_fen("9/9/9/9/9/9/9/9/9/8 w - - 0 1"),
            Err(FenError::RowWidth { .. })
        ));
        // 缺少红方帅
        assert!(matches!(
            from_fen("4k4/9/9/9/9/9/9/9/9/9 w - - 0 1"),
            Err(FenError::BadPosition(PositionError::MissingKing(
                Color::Red
            )))
        ));
        // 将帅不在九宫（黑将在 e6 —— 黑方九宫是 row 7..9）
        assert!(matches!(
            from_fen("9/9/9/4k4/9/9/9/9/9/4K4 w - - 0 1"),
            Err(FenError::KingOutsidePalace(Color::Black))
        ));
        // 非法字符
        assert!(matches!(
            from_fen("9/9/9/9/9/9/9/9/9/x8 w - - 0 1"),
            Err(FenError::BadChar { .. })
        ));
    }

    /// 子力越界不应 panic。
    #[test]
    fn board_with_no_pieces_errors_cleanly() {
        assert!(from_fen("9/9/9/9/9/9/9/9/9/9 w - - 0 1").is_err());
    }
}
