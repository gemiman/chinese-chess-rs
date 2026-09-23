//! 棋子种类与格子编码。
//!
//! # 格子编码
//!
//! 棋盘上每一格的取值是一个 `u8`：
//!
//! | 取值 | 含义 |
//! |---|---|
//! | `0` | 空格 |
//! | `1..=7` | 红方棋子（`= kind`） |
//! | `9..=15` | 黑方棋子（`= kind \| 8`） |
//!
//! 即 `piece = kind as u8 | color.bit()`。这样「是否为空」只需一次 `== 0` 比较，
//! 比 `Option<T>` 省一次分支，且可直接作为数组索引使用。
//!
//! > 命名说明：设计文档 [docs/03](../../../docs/03-规则引擎与领域模型.md) 用
//! > `pub type Square = u8` 表示这一编码。本项目改用 `Piece` 语义 + 裸 `u8`，
//! > 因为「Square」一词在棋类编程里通常指**格子索引**，两者混用极易出 bug。

use crate::color::Color;

/// 空格。
pub const EMPTY: u8 = 0;

/// 棋子种类。判别值即编码中的低 3 位。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum PieceKind {
    /// 帅 / 将
    King = 1,
    /// 仕 / 士
    Advisor = 2,
    /// 相 / 象
    Elephant = 3,
    /// 马
    Horse = 4,
    /// 车
    Chariot = 5,
    /// 炮
    Cannon = 6,
    /// 兵 / 卒
    Pawn = 7,
}

impl PieceKind {
    /// 全部七种棋子，按判别值升序。
    pub const ALL: [PieceKind; 7] = [
        PieceKind::King,
        PieceKind::Advisor,
        PieceKind::Elephant,
        PieceKind::Horse,
        PieceKind::Chariot,
        PieceKind::Cannon,
        PieceKind::Pawn,
    ];

    /// 由低 3 位码值还原棋子种类。
    #[inline]
    pub const fn from_code(code: u8) -> Option<PieceKind> {
        match code & 7 {
            1 => Some(PieceKind::King),
            2 => Some(PieceKind::Advisor),
            3 => Some(PieceKind::Elephant),
            4 => Some(PieceKind::Horse),
            5 => Some(PieceKind::Chariot),
            6 => Some(PieceKind::Cannon),
            7 => Some(PieceKind::Pawn),
            _ => None,
        }
    }

    /// FEN 字母（红方大写、黑方小写）。
    pub const fn fen_char(self, color: Color) -> char {
        match (self, color) {
            (PieceKind::King, Color::Red) => 'K',
            (PieceKind::King, Color::Black) => 'k',
            (PieceKind::Advisor, Color::Red) => 'A',
            (PieceKind::Advisor, Color::Black) => 'a',
            (PieceKind::Elephant, Color::Red) => 'B',
            (PieceKind::Elephant, Color::Black) => 'b',
            (PieceKind::Horse, Color::Red) => 'N',
            (PieceKind::Horse, Color::Black) => 'n',
            (PieceKind::Chariot, Color::Red) => 'R',
            (PieceKind::Chariot, Color::Black) => 'r',
            (PieceKind::Cannon, Color::Red) => 'C',
            (PieceKind::Cannon, Color::Black) => 'c',
            (PieceKind::Pawn, Color::Red) => 'P',
            (PieceKind::Pawn, Color::Black) => 'p',
        }
    }

    /// 从 FEN 字母还原「棋子种类 + 颜色」。
    pub const fn from_fen_char(ch: char) -> Option<(PieceKind, Color)> {
        let (kind, color) = match ch {
            'K' => (PieceKind::King, Color::Red),
            'k' => (PieceKind::King, Color::Black),
            'A' => (PieceKind::Advisor, Color::Red),
            'a' => (PieceKind::Advisor, Color::Black),
            'B' => (PieceKind::Elephant, Color::Red),
            'b' => (PieceKind::Elephant, Color::Black),
            'N' => (PieceKind::Horse, Color::Red),
            'n' => (PieceKind::Horse, Color::Black),
            'R' => (PieceKind::Chariot, Color::Red),
            'r' => (PieceKind::Chariot, Color::Black),
            'C' => (PieceKind::Cannon, Color::Red),
            'c' => (PieceKind::Cannon, Color::Black),
            'P' => (PieceKind::Pawn, Color::Red),
            'p' => (PieceKind::Pawn, Color::Black),
            _ => return None,
        };
        Some((kind, color))
    }

    /// 中文名称。红黑双方字形各不相同，采用传统异形字：
    /// 红 `帥仕相傌俥炮兵`，黑 `將士象馬車砲卒`。
    pub const fn name_zh(self, color: Color) -> char {
        match (self, color) {
            (PieceKind::King, Color::Red) => '帥',
            (PieceKind::King, Color::Black) => '將',
            (PieceKind::Advisor, Color::Red) => '仕',
            (PieceKind::Advisor, Color::Black) => '士',
            (PieceKind::Elephant, Color::Red) => '相',
            (PieceKind::Elephant, Color::Black) => '象',
            (PieceKind::Horse, Color::Red) => '傌',
            (PieceKind::Horse, Color::Black) => '馬',
            (PieceKind::Chariot, Color::Red) => '俥',
            (PieceKind::Chariot, Color::Black) => '車',
            (PieceKind::Cannon, Color::Red) => '炮',
            (PieceKind::Cannon, Color::Black) => '砲',
            (PieceKind::Pawn, Color::Red) => '兵',
            (PieceKind::Pawn, Color::Black) => '卒',
        }
    }

    /// 从中文棋子字还原「棋子种类 + 颜色」。
    ///
    /// 颜色返回 `None` 表示该字形红黑同字，需由当前走子方决定 —— 只有通用简体
    /// 写法 `车` `马` 如此，保留它们是为了容忍不区分异形字的输入习惯。
    /// 其余字形（含 `帥` `將` `俥` `車` `傌` `馬` `炮` `砲`）自带颜色。
    pub const fn from_name_zh(ch: char) -> Option<(PieceKind, Option<Color>)> {
        let r = match ch {
            '帥' => (PieceKind::King, Some(Color::Red)),
            '將' => (PieceKind::King, Some(Color::Black)),
            '仕' => (PieceKind::Advisor, Some(Color::Red)),
            '士' => (PieceKind::Advisor, Some(Color::Black)),
            '相' => (PieceKind::Elephant, Some(Color::Red)),
            '象' => (PieceKind::Elephant, Some(Color::Black)),
            '傌' => (PieceKind::Horse, Some(Color::Red)),
            '馬' => (PieceKind::Horse, Some(Color::Black)),
            '俥' => (PieceKind::Chariot, Some(Color::Red)),
            '車' => (PieceKind::Chariot, Some(Color::Black)),
            '炮' => (PieceKind::Cannon, Some(Color::Red)),
            '砲' => (PieceKind::Cannon, Some(Color::Black)),
            '兵' => (PieceKind::Pawn, Some(Color::Red)),
            '卒' => (PieceKind::Pawn, Some(Color::Black)),
            // 简体通用写法：红黑同字，交由走子方决定颜色
            '车' => (PieceKind::Chariot, None),
            '马' => (PieceKind::Horse, None),
            _ => return None,
        };
        Some(r)
    }

    /// 该棋子是否按斜线前进（走法上纵向与横向位移均为固定值）。
    ///
    /// 用于中文记谱：斜行棋子的「进 / 退」后跟**目标路数**，
    /// 直行棋子跟**移动格数**。详见 [`crate::notation`]。
    #[inline]
    pub const fn moves_diagonally(self) -> bool {
        matches!(
            self,
            PieceKind::Horse | PieceKind::Elephant | PieceKind::Advisor
        )
    }
}

/// 组合一颗棋子。
#[inline]
pub const fn encode(color: Color, kind: PieceKind) -> u8 {
    (kind as u8) | color.bit()
}

/// 该格是否为空。
#[inline]
pub const fn is_empty(piece: u8) -> bool {
    piece == EMPTY
}

/// 取出棋子颜色；空格返回 `None`。
#[inline]
pub const fn color_of(piece: u8) -> Option<Color> {
    if piece == EMPTY {
        None
    } else if piece & 8 != 0 {
        Some(Color::Black)
    } else {
        Some(Color::Red)
    }
}

/// 取出棋子种类；空格返回 `None`。
#[inline]
pub const fn kind_of(piece: u8) -> Option<PieceKind> {
    PieceKind::from_code(piece)
}

/// Zobrist 表用的棋子索引，值域 `0..14`：红 `0..7`、黑 `7..14`。
///
/// 传入空格（0）是调用方 bug，会触发下溢 panic。
#[inline]
pub const fn zobrist_index(piece: u8) -> usize {
    ((piece & 7) - 1) as usize + ((piece >> 3) as usize) * 7
}
