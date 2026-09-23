//! 走子方颜色。

/// 对局双方。
///
/// `Red` 的判别值为 0、`Black` 为 1，与 [`crate::piece`] 的格子编码中
/// 「color 占第 4 位」的约定一致：`piece = kind | ((color as u8) << 3)`。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[repr(u8)]
pub enum Color {
    /// 红方。棋盘 `row 0` 一侧，先行。
    Red = 0,
    /// 黑方。棋盘 `row 9` 一侧，后行。
    Black = 1,
}

impl Color {
    /// 本颜色在格子编码中占用的位偏移量（红 = 0，黑 = 8）。
    #[inline]
    pub const fn bit(self) -> u8 {
        (self as u8) << 3
    }

    /// 对方。
    #[inline]
    pub const fn opponent(self) -> Color {
        match self {
            Color::Red => Color::Black,
            Color::Black => Color::Red,
        }
    }

    /// 本方「前进」方向对应的 `row` 增量。
    ///
    /// 红方底线是 `row 0`，故红方前进为 `+1`；黑方底线是 `row 9`，故前进为 `-1`。
    #[inline]
    pub const fn forward(self) -> i8 {
        match self {
            Color::Red => 1,
            Color::Black => -1,
        }
    }

    /// 本方九宫对应的 `row` 区间（闭区间）。
    #[inline]
    pub const fn palace_rows(self) -> (u8, u8) {
        match self {
            Color::Red => (0, 2),
            Color::Black => (7, 9),
        }
    }

    /// 中文名称，用于错误信息与日志。
    #[inline]
    pub const fn name_zh(self) -> &'static str {
        match self {
            Color::Red => "红",
            Color::Black => "黑",
        }
    }

    /// FEN 中的走子方标记。
    #[inline]
    pub const fn fen_char(self) -> char {
        match self {
            Color::Red => 'w',
            Color::Black => 'b',
        }
    }

    /// 从 FEN 标记解析走子方。
    #[inline]
    pub const fn from_fen_char(ch: char) -> Option<Color> {
        match ch {
            'w' | 'W' | 'r' | 'R' => Some(Color::Red),
            'b' | 'B' => Some(Color::Black),
            _ => None,
        }
    }
}
