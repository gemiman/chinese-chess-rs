//! 坐标换算：`(col, row)` ↔ 一维索引 ↔ ICCS 字符串 ↔ 中文路数。
//!
//! # 坐标约定（[ADR-013](../../../docs/14-决策记录ADR.md#adr-013)）
//!
//! ```text
//!        a    b    c    d    e    f    g    h    i      ← col 0..8
//!  row 9│ 車 │ 馬 │ 象 │ 士 │ 將 │ 士 │ 象 │ 馬 │ 車 │  黑方底线
//!  ...
//!  row 0│ 车 │ 马 │ 相 │ 仕 │ 帅 │ 仕 │ 相 │ 马 │ 车 │  红方底线
//! ```
//!
//! - `row 0` = **红方底线**，`row 9` = **黑方底线**
//! - 红方前进 = `row` 递增；黑方前进 = `row` 递减
//! - 一维索引 `index = row * 9 + col`
//!
//! > ⚠️ 渲染时必须注意：SVG / 屏幕的 y 轴向下，而棋理的 `row 0` 在视觉**下方**。
//! > 渲染坐标需做 `(ROWS - 1 - row)` 翻转，`prototype/build_prototype.py` 中已踩过这个坑。

use crate::color::Color;

/// 列数。
pub const COLS: u8 = 9;
/// 行数。
pub const ROWS: u8 = 10;
/// 棋盘总格数。
pub const BOARD_SIZE: usize = (COLS as usize) * (ROWS as usize);

/// 四个正交方向 `(Δcol, Δrow)`。
pub const ORTHO: [(i8, i8); 4] = [(0, 1), (0, -1), (1, 0), (-1, 0)];

/// 四个斜向方向 `(Δcol, Δrow)`。
pub const DIAG: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

/// 由 `(col, row)` 计算一维索引。
#[inline]
pub const fn index(col: u8, row: u8) -> u8 {
    row * COLS + col
}

/// 由一维索引取列。
#[inline]
pub const fn col_of(idx: u8) -> u8 {
    idx % COLS
}

/// 由一维索引取行。
#[inline]
pub const fn row_of(idx: u8) -> u8 {
    idx / COLS
}

/// 该坐标是否在棋盘内。
#[inline]
pub const fn on_board(col: i8, row: i8) -> bool {
    col >= 0 && col < COLS as i8 && row >= 0 && row < ROWS as i8
}

/// 索引是否合法。
#[inline]
pub const fn is_valid_index(idx: u8) -> bool {
    (idx as usize) < BOARD_SIZE
}

/// 该格是否位于 `color` 方的九宫内。
#[inline]
pub const fn in_palace(color: Color, col: u8, row: u8) -> bool {
    if col < 3 || col > 5 {
        return false;
    }
    let (lo, hi) = color.palace_rows();
    row >= lo && row <= hi
}

/// 该行是否属于 `color` 方的半场（相 / 象不过河的判据）。
#[inline]
pub const fn on_own_side(color: Color, row: u8) -> bool {
    match color {
        Color::Red => row <= 4,
        Color::Black => row >= 5,
    }
}

/// 该行是否已过河（兵 / 卒横向移动能力的判据）。
///
/// 「过河」指棋子位于**对方**半场。
#[inline]
pub const fn has_crossed_river(color: Color, row: u8) -> bool {
    !on_own_side(color, row)
}

/// 中文记谱路数。
///
/// 红黑双方各自从**自己的右侧**数 1..9 路：
/// - 红方：`route = 9 - col`（列 `i` → 1 路，列 `a` → 9 路）
/// - 黑方：`route = col + 1`（列 `a` → 1 路，列 `i` → 9 路）
#[inline]
pub const fn route_number(color: Color, col: u8) -> u8 {
    match color {
        Color::Red => 9 - col,
        Color::Black => col + 1,
    }
}

/// [`route_number`] 的逆运算：由路数还原列号。
#[inline]
pub const fn col_from_route(color: Color, route: u8) -> Option<u8> {
    if route < 1 || route > 9 {
        return None;
    }
    Some(match color {
        Color::Red => 9 - route,
        Color::Black => route - 1,
    })
}

/// 一维索引转 ICCS 字符串（列字母 + 行数字），如 `index(7, 2) == 25` → `"h2"`。
pub fn to_iccs(idx: u8) -> String {
    let col = col_of(idx);
    let row = row_of(idx);
    let mut s = String::with_capacity(2);
    s.push((b'a' + col) as char);
    s.push((b'0' + row) as char);
    s
}

/// ICCS 字符串转一维索引，如 `"h2"` → `index(7, 2) == 25`。
pub fn from_iccs(s: &str) -> Option<u8> {
    let bytes = s.as_bytes();
    if bytes.len() != 2 {
        return None;
    }
    let col = bytes[0];
    let row = bytes[1];
    if !(b'a'..=b'i').contains(&col) || !row.is_ascii_digit() {
        return None;
    }
    Some(index(col - b'a', row - b'0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_roundtrip_is_total() {
        for idx in 0..BOARD_SIZE as u8 {
            assert_eq!(index(col_of(idx), row_of(idx)), idx);
        }
    }

    #[test]
    fn iccs_roundtrip_is_total() {
        for idx in 0..BOARD_SIZE as u8 {
            let s = to_iccs(idx);
            assert_eq!(from_iccs(&s), Some(idx), "ICCS 往返失败: {s}");
        }
    }

    #[test]
    fn iccs_known_values() {
        assert_eq!(to_iccs(index(7, 2)), "h2");
        assert_eq!(to_iccs(index(4, 0)), "e0");
        assert_eq!(to_iccs(index(0, 9)), "a9");
        assert_eq!(from_iccs("h2"), Some(index(7, 2)));
        assert_eq!(from_iccs("j0"), None);
        assert_eq!(from_iccs("h10"), None);
        assert_eq!(from_iccs(""), None);
    }

    /// 设计文档 §2.3 的双向验证：红炮在 `h2` 是「二路」，黑炮在 `h7` 是「8 路」。
    #[test]
    fn route_number_matches_design_doc() {
        // 红方：列 i(8) → 1 路；列 a(0) → 9 路
        assert_eq!(route_number(Color::Red, 8), 1);
        assert_eq!(route_number(Color::Red, 0), 9);
        // 红炮初始在 h 列 → 7 → 9-7 = 2 路
        assert_eq!(route_number(Color::Red, 7), 2);
        // 黑方：列 a(0) → 1 路；列 i(8) → 9 路
        assert_eq!(route_number(Color::Black, 0), 1);
        assert_eq!(route_number(Color::Black, 8), 9);
        // 黑炮初始在 h 列 → 7 → 7+1 = 8 路
        assert_eq!(route_number(Color::Black, 7), 8);
    }

    #[test]
    fn route_col_roundtrip() {
        for color in [Color::Red, Color::Black] {
            for col in 0..COLS {
                let route = route_number(color, col);
                assert_eq!(col_from_route(color, route), Some(col));
            }
        }
    }

    #[test]
    fn palace_and_river_bounds() {
        assert!(in_palace(Color::Red, 4, 1));
        assert!(in_palace(Color::Red, 3, 2));
        assert!(!in_palace(Color::Red, 3, 3));
        assert!(!in_palace(Color::Red, 6, 1));
        assert!(in_palace(Color::Black, 5, 9));
        assert!(!in_palace(Color::Black, 5, 6));

        assert!(on_own_side(Color::Red, 4));
        assert!(!on_own_side(Color::Red, 5));
        assert!(has_crossed_river(Color::Red, 5));
        assert!(on_own_side(Color::Black, 5));
        assert!(has_crossed_river(Color::Black, 4));
    }
}
