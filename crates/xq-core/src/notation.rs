//! 中文记谱 ↔ `Move` 转换。
//!
//! # 格式
//!
//! ```text
//! <棋子名><起点路数><动作><目标>        例如：炮二平五
//! <前/中/后/序数><棋子名><动作><目标>    例如：前车进一
//! ```
//!
//! # 最容易错的两件事
//!
//! **① 「进 / 退」后面跟的是格数还是路数？** 取决于棋子：
//!
//! | 棋子 | 进 / 退 后的数字含义 |
//! |---|---|
//! | 车、炮、兵/卒、**帅/将** | **移动的格数** |
//! | 马、相/象、仕/士 | **目标路数** |
//!
//! > ⚠️ 设计文档 [docs/03](../../../docs/03-规则引擎与领域模型.md) §9.4 的表把
//! > 「帅 / 将」归入了「目标路数」一档，这是**错误的**。将帅只走直向一步：
//! > 纵向移动时起止路数相同，记谱必然是「帅五进一」而不是「帅五进五」。
//! > 本文档已在实现中修正，并回写了设计文档。
//!
//! **② 数字字形**：红方用汉字数字（一二三四五六七八九），黑方用阿拉伯数字（1-9）。
//! 解析时**两种都接受**，输出时按上述规范。
//!
//! # 消歧（前 / 中 / 后）
//!
//! 当**同一条竖线上**存在多个同种同色棋子时，路数无法唯一标识起点，改用序数：
//!
//! | 同线子数 | 记法 |
//! |---|---|
//! | 1 | 用路数 |
//! | 2 | 前 / 后 |
//! | 3 | 前 / 中 / 后 |
//! | ≥4 | 前 / 二 / 三 / … / 后 |
//!
//! 「前」的判定：红方 `row` 大者为前，黑方 `row` 小者为前（各以对方底线为前方）。

use crate::color::Color;
use crate::mv::{Move, MoveList};
use crate::piece::{PieceKind, color_of, kind_of};
use crate::position::Position;
use crate::square::{COLS, ROWS, col_of, index, route_number, row_of};

/// 记谱动作。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// 进：向对方底线方向移动。
    Advance,
    /// 退：向己方底线方向移动。
    Retreat,
    /// 平：横向移动（同一行）。
    Traverse,
}

impl Action {
    /// 对应的汉字。
    pub const fn char(self) -> char {
        match self {
            Action::Advance => '进',
            Action::Retreat => '退',
            Action::Traverse => '平',
        }
    }

    /// 从汉字解析。
    pub const fn from_char(ch: char) -> Option<Action> {
        match ch {
            '进' => Some(Action::Advance),
            '退' => Some(Action::Retreat),
            '平' => Some(Action::Traverse),
            _ => None,
        }
    }
}

/// 起点的标识方式。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Subject {
    /// 用路数（1..=9）标识。
    Route(u8),
    /// 「前」。
    Front,
    /// 「后」。
    Back,
    /// 「中」（仅同线 3 子时使用）。
    Middle,
    /// 序数「二」「三」…（同线 ≥4 子时的中间子，值为该序数本身）。
    Nth(u8),
}

/// 一步棋的记谱要素。用于生成与解析的比对。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MoveDesc {
    /// 走子方。
    pub color: Color,
    /// 棋子种类。
    pub kind: PieceKind,
    /// 起点标识。
    pub subject: Subject,
    /// 动作。
    pub action: Action,
    /// 动作后的数字（含义见模块文档）。
    pub value: u8,
}

/// 记谱解析错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotationError {
    /// 空字符串。
    Empty,
    /// 无法识别的棋子字。
    UnknownPieceChar(char),
    /// 缺少棋子字。
    MissingPieceChar,
    /// 缺少路数。
    MissingRoute,
    /// 缺少动作字。
    MissingAction,
    /// 无法识别的动作字。
    UnknownAction(char),
    /// 缺少动作后的数字。
    MissingValue,
    /// 数字超出 1..=9。
    ValueOutOfRange(u8),
    /// `describe` 收到的着法起点没有棋子。
    NoPieceAtFrom,
    /// 当前局面下没有任何合法着法匹配该记谱。
    NoMatch,
    /// 有多个合法着法匹配（记谱本身有歧义）。
    Ambiguous,
}

impl core::fmt::Display for NotationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            NotationError::Empty => write!(f, "记谱为空"),
            NotationError::UnknownPieceChar(c) => write!(f, "无法识别的棋子字: {c}"),
            NotationError::MissingPieceChar => write!(f, "缺少棋子字"),
            NotationError::MissingRoute => write!(f, "缺少起点路数"),
            NotationError::MissingAction => write!(f, "缺少动作字（进/退/平）"),
            NotationError::UnknownAction(c) => write!(f, "无法识别的动作字: {c}"),
            NotationError::MissingValue => write!(f, "缺少动作后的数字"),
            NotationError::ValueOutOfRange(n) => write!(f, "路数或格数超出 1..=9: {n}"),
            NotationError::NoPieceAtFrom => write!(f, "着法起点没有棋子"),
            NotationError::NoMatch => write!(f, "当前局面下没有合法着法匹配该记谱"),
            NotationError::Ambiguous => write!(f, "该记谱在当前局面下匹配到多个合法着法"),
        }
    }
}

impl std::error::Error for NotationError {}

/// 汉字数字。
const CN_DIGITS: [char; 9] = ['一', '二', '三', '四', '五', '六', '七', '八', '九'];

/// 按颜色规范输出数字：红方汉字、黑方阿拉伯数字。
fn numeral(color: Color, n: u8) -> Result<String, NotationError> {
    if !(1..=9).contains(&n) {
        return Err(NotationError::ValueOutOfRange(n));
    }
    Ok(match color {
        Color::Red => CN_DIGITS[(n - 1) as usize].to_string(),
        Color::Black => ((b'0' + n) as char).to_string(),
    })
}

/// 解析数字，汉字与阿拉伯数字均可。
fn parse_numeral(ch: Option<char>) -> Option<u8> {
    let ch = ch?;
    if let Some(i) = CN_DIGITS.iter().position(|&c| c == ch) {
        return Some(i as u8 + 1);
    }
    if ('1'..='9').contains(&ch) {
        return Some(ch as u8 - b'0');
    }
    None
}

/// 解析消歧前缀。
fn parse_disambig(ch: char) -> Option<Subject> {
    match ch {
        '前' => Some(Subject::Front),
        '后' => Some(Subject::Back),
        '中' => Some(Subject::Middle),
        _ => parse_numeral(Some(ch)).and_then(|n| (n >= 2).then_some(Subject::Nth(n))),
    }
}

impl Position {
    /// 同一条竖线上某方某种棋子的全部位置，按**从「前」到「后」**排列。
    fn column_pieces_ordered(&self, col: u8, color: Color, kind: PieceKind) -> Vec<u8> {
        let mut v = Vec::new();
        for row in 0..ROWS {
            let idx = index(col, row);
            let p = self.piece_at(idx);
            if color_of(p) == Some(color) && kind_of(p) == Some(kind) {
                v.push(idx);
            }
        }
        // 升序得到「row 小 → 大」。红方 row 大者为前，故需反转；黑方 row 小者为前，保持。
        if color == Color::Red {
            v.reverse();
        }
        v
    }

    /// 把一步棋拆解为记谱要素。
    pub fn describe(&self, mv: Move) -> Result<MoveDesc, NotationError> {
        let from = mv.from();
        let to = mv.to();
        let piece = self.piece_at(from);
        let color = color_of(piece).ok_or(NotationError::NoPieceAtFrom)?;
        let kind = kind_of(piece).ok_or(NotationError::NoPieceAtFrom)?;

        // ---- 起点标识 ----
        let col = col_of(from);
        let same_line = self.column_pieces_ordered(col, color, kind);
        let subject = if same_line.len() <= 1 {
            Subject::Route(route_number(color, col))
        } else {
            let i = same_line
                .iter()
                .position(|&x| x == from)
                .ok_or(NotationError::NoPieceAtFrom)?;
            let n = same_line.len();
            if i == 0 {
                Subject::Front
            } else if i == n - 1 {
                Subject::Back
            } else if n == 3 {
                Subject::Middle
            } else {
                Subject::Nth((i + 1) as u8)
            }
        };

        // ---- 动作与数值 ----
        let from_row = row_of(from);
        let to_row = row_of(to);
        let (action, value) = if from_row == to_row {
            (Action::Traverse, route_number(color, col_of(to)))
        } else {
            let delta = to_row as i8 - from_row as i8;
            let is_forward = delta * color.forward() > 0;
            let action = if is_forward {
                Action::Advance
            } else {
                Action::Retreat
            };
            let value = if kind.moves_diagonally() {
                // 马 / 相 / 仕：跟目标路数
                route_number(color, col_of(to))
            } else {
                // 车 / 炮 / 兵卒 / 将帅：跟移动格数
                delta.unsigned_abs()
            };
            (action, value)
        };

        Ok(MoveDesc {
            color,
            kind,
            subject,
            action,
            value,
        })
    }

    /// 渲染记谱要素为字符串。
    fn render_desc(&self, d: MoveDesc) -> Result<String, NotationError> {
        let mut s = String::with_capacity(8);
        let glyph = d.kind.name_zh(d.color);
        match d.subject {
            Subject::Route(route) => {
                s.push(glyph);
                s.push_str(&numeral(d.color, route)?);
            }
            Subject::Front => {
                s.push('前');
                s.push(glyph);
            }
            Subject::Back => {
                s.push('后');
                s.push(glyph);
            }
            Subject::Middle => {
                s.push('中');
                s.push(glyph);
            }
            Subject::Nth(n) => {
                s.push_str(&numeral(d.color, n)?);
                s.push(glyph);
            }
        }
        s.push(d.action.char());
        s.push_str(&numeral(d.color, d.value)?);
        Ok(s)
    }

    /// 把一步棋转成中文记谱。
    pub fn to_chinese_notation(&self, mv: Move) -> Result<String, NotationError> {
        let desc = self.describe(mv)?;
        self.render_desc(desc)
    }

    /// 把一步棋转成 ICCS 串（如 `h2e2`），便于日志与调试。
    pub fn to_iccs_string(&self, mv: Move) -> String {
        format!(
            "{}{}",
            crate::square::to_iccs(mv.from()),
            crate::square::to_iccs(mv.to())
        )
    }

    /// 解析一步中文记谱，返回对应的合法着法。
    pub fn from_chinese_notation(&mut self, text: &str) -> Result<Move, NotationError> {
        let want = parse_desc(text, self.side_to_move())?;

        let mut list = MoveList::new();
        self.gen_legal_into(&mut list);

        let mut found: Option<Move> = None;
        for i in 0..list.len() {
            let mv = list.get(i);
            // 2024 edition 的 let-chain：同时满足「能解析出要素」与「要素相等」
            if let Ok(d) = self.describe(mv)
                && d == want
            {
                if found.is_some() {
                    return Err(NotationError::Ambiguous);
                }
                found = Some(mv);
            }
        }
        found.ok_or(NotationError::NoMatch)
    }
}

/// 解析记谱文本为要素。`side` 是当前走子方，用于确定红黑。
fn parse_desc(text: &str, side: Color) -> Result<MoveDesc, NotationError> {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.is_empty() {
        return Err(NotationError::Empty);
    }

    let mut i: usize;

    // 校验棋子字与当前走子方是否匹配。马 / 车 / 炮 红黑同字，不做颜色校验；
    // 帅/将、仕/士、相/象、兵/卒 各有专属字形，写错即视为非法记谱 ——
    // 否则「帅五进一」会被误匹配到黑方的将。
    let check_glyph =
        |kind: PieceKind, glyph_color: Color, ch: char| -> Result<(), NotationError> {
            if kind.is_color_agnostic_glyph() || glyph_color == side {
                Ok(())
            } else {
                Err(NotationError::UnknownPieceChar(ch))
            }
        };

    // ---- 主体：棋子字 + 路数，或 序数 + 棋子字 ----
    let (subject, kind) = match parse_disambig(chars[0]) {
        // 首字是消歧前缀（前/中/后/序数），次字必须是棋子字
        Some(sub) => {
            i = 1;
            let ch = *chars.get(i).ok_or(NotationError::MissingPieceChar)?;
            let (kind, glyph_color) =
                PieceKind::from_name_zh(ch).ok_or(NotationError::UnknownPieceChar(ch))?;
            check_glyph(kind, glyph_color, ch)?;
            i += 1;
            (sub, kind)
        }
        // 首字是棋子字，次字是路数
        None => {
            let ch = chars[0];
            let (kind, glyph_color) =
                PieceKind::from_name_zh(ch).ok_or(NotationError::UnknownPieceChar(ch))?;
            check_glyph(kind, glyph_color, ch)?;
            i = 1;
            let route = parse_numeral(chars.get(i).copied()).ok_or(NotationError::MissingRoute)?;
            if !(1..=9).contains(&route) {
                return Err(NotationError::ValueOutOfRange(route));
            }
            i += 1;
            (Subject::Route(route), kind)
        }
    };

    // ---- 动作 ----
    let action_ch = *chars.get(i).ok_or(NotationError::MissingAction)?;
    let action = Action::from_char(action_ch).ok_or(NotationError::UnknownAction(action_ch))?;
    i += 1;

    // ---- 数值 ----
    let value = parse_numeral(chars.get(i).copied()).ok_or(NotationError::MissingValue)?;
    if !(1..=9).contains(&value) {
        return Err(NotationError::ValueOutOfRange(value));
    }

    Ok(MoveDesc {
        color: side,
        kind,
        subject,
        action,
        value,
    })
}

/// 编译期护栏：棋盘点数与记谱路数范围自洽。
const _: () = {
    assert!(COLS == 9, "中文记谱的路数范围 1..=9 依赖 9 列");
    assert!(ROWS == 10);
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::square::from_iccs;

    fn notation_of(fen: &str, iccs: &str) -> String {
        let pos = crate::fen::from_fen(fen).expect("FEN 解析失败");
        let mv = Move::new(
            from_iccs(&iccs[0..2]).unwrap(),
            from_iccs(&iccs[2..4]).unwrap(),
        );
        pos.to_chinese_notation(mv).expect("生成记谱失败")
    }

    const START: &str = crate::fen::STARTPOS_FEN;

    /// 设计文档 §2.3 / §9.4 的全部示例。
    #[test]
    fn doc_examples() {
        // 红方「炮二平五」= h2 → e2
        assert_eq!(notation_of(START, "h2e2"), "炮二平五");
        // 红方「马二进三」= h0 → g2（跟目标路数，不是格数）
        assert_eq!(notation_of(START, "h0g2"), "马二进三");
        // 红方「炮八平五」= b2 → e2
        assert_eq!(notation_of(START, "b2e2"), "炮八平五");
        // 红方「马八进七」= b0 → c2
        assert_eq!(notation_of(START, "b0c2"), "马八进七");
        // 红方「兵七进一」= c3 → c4（跟格数）
        assert_eq!(notation_of(START, "c3c4"), "兵七进一");
        // 红方「相三进五」= g0 → e2（跟目标路数）
        assert_eq!(notation_of(START, "g0e2"), "相三进五");
        // 红方「仕四进五」= f0 → e1（跟目标路数）
        assert_eq!(notation_of(START, "f0e1"), "仕四进五");
        // 红方「帅五进一」= e0 → e1（跟格数 —— 若按「目标路数」会输出「帅五进五」）
        assert_eq!(notation_of(START, "e0e1"), "帅五进一");
        // 红方车：a0 是九路，前进一格
        assert_eq!(notation_of(START, "a0a1"), "车九进一");
        assert_eq!(notation_of(START, "i0i1"), "车一进一");
    }

    /// 黑方用阿拉伯数字（路数与步数都是）。
    #[test]
    fn black_uses_arabic_numerals() {
        // 黑炮 h7 是 8 路，平到 e7（5 路）
        assert_eq!(notation_of(START, "h7e7"), "炮8平5");
        // 黑马 h9 是 8 路，进到 g7（7 路）—— 这是应对当头炮的标准着法
        assert_eq!(notation_of(START, "h9g7"), "马8进7");
        // 黑卒 a6 是 1 路，前进一格
        assert_eq!(notation_of(START, "a6a5"), "卒1进1");
    }

    /// 「进 / 退」的方向判定红黑相反：黑方 `row` 减小才是「进」。
    #[test]
    fn advance_direction_differs_by_side() {
        // 黑卒 a6 → a5：黑方前进方向是 row 递减，故为「进」
        assert_eq!(notation_of(START, "a6a5"), "卒1进1");

        // 黑车在 a7；红兵 e4 封住 e 列，避免两王照面
        let fen = "4k4/9/r8/9/9/4P4/9/9/9/4K4 b - - 0 1";
        let pos = crate::fen::from_fen(fen).expect("FEN 应可解析");
        // a7 → a8：对黑方是后退（row 增大）
        let mv_back = Move::new(from_iccs("a7").unwrap(), from_iccs("a8").unwrap());
        assert_eq!(pos.to_chinese_notation(mv_back).unwrap(), "车1退1");
        // a7 → a6：向前走一格
        let mv_fwd = Move::new(from_iccs("a7").unwrap(), from_iccs("a6").unwrap());
        assert_eq!(pos.to_chinese_notation(mv_fwd).unwrap(), "车1进1");
    }

    /// 同线两子用「前 / 后」（红方 row 大者为前）。
    #[test]
    fn front_back_disambiguation_with_two_pieces() {
        // 红方两车同在 e 列：e3（row 大 → 前）、e1（后）
        let fen = "4k4/9/9/9/9/9/4R4/9/4R4/4K4 w - - 0 1";
        let pos = crate::fen::from_fen(fen).unwrap();

        let front = Move::new(from_iccs("e3").unwrap(), from_iccs("e4").unwrap());
        assert_eq!(pos.to_chinese_notation(front).unwrap(), "前车进一");

        let back = Move::new(from_iccs("e1").unwrap(), from_iccs("e2").unwrap());
        assert_eq!(pos.to_chinese_notation(back).unwrap(), "后车进一");
    }

    /// 同线三子用「前 / 中 / 后」。
    #[test]
    fn middle_disambiguation_with_three_pieces() {
        // e5（前）、e3（中）、e1（后）
        let fen = "4k4/9/9/9/4R4/9/4R4/9/4R4/4K4 w - - 0 1";
        let pos = crate::fen::from_fen(fen).unwrap();

        let front = Move::new(from_iccs("e5").unwrap(), from_iccs("e6").unwrap());
        assert_eq!(pos.to_chinese_notation(front).unwrap(), "前车进一");

        let middle = Move::new(from_iccs("e3").unwrap(), from_iccs("e4").unwrap());
        assert_eq!(pos.to_chinese_notation(middle).unwrap(), "中车进一");

        let back = Move::new(from_iccs("e1").unwrap(), from_iccs("e2").unwrap());
        assert_eq!(pos.to_chinese_notation(back).unwrap(), "后车进一");
    }

    /// 同线五子用「前 / 二 / 三 / 四 / 后」。
    #[test]
    fn nth_disambiguation_with_five_pawns_on_one_file() {
        // a 列五个红兵，row 依次为 7/6/5/4/3（row 大者为前）
        let fen = "3k5/9/P8/P8/P8/P8/P8/9/9/4K4 w - - 0 1";
        let pos = crate::fen::from_fen(fen).unwrap();

        // describe 不校验合法性，故可用任意 from→to 探测起点标识
        let subject_of = |coord: &str| -> String {
            let from = from_iccs(coord).unwrap();
            let to = from + 9; // 向 row 增大方向前进一格
            pos.to_chinese_notation(Move::new(from, to))
                .unwrap_or_else(|e| panic!("{coord} 记谱生成失败: {e}"))
        };

        assert_eq!(subject_of("a7"), "前兵进一");
        assert_eq!(subject_of("a6"), "二兵进一");
        assert_eq!(subject_of("a5"), "三兵进一");
        assert_eq!(subject_of("a4"), "四兵进一");
        assert_eq!(subject_of("a3"), "后兵进一");
    }

    /// 不同线的同种棋子不触发消歧，仍用路数。
    #[test]
    fn pieces_on_different_files_use_route_number() {
        // 初始局面两个红炮在 b2 / h2，不同列
        let pos = crate::fen::from_fen(START).unwrap();
        let mv = Move::new(from_iccs("b2").unwrap(), from_iccs("e2").unwrap());
        assert_eq!(pos.to_chinese_notation(mv).unwrap(), "炮八平五");
    }

    /// 繁体异体字也能解析。
    #[test]
    fn traditional_glyphs_are_accepted() {
        let mut pos = Position::startpos();
        // 「馬二进三」是繁体的马二进三（动作字仍用简体）
        assert!(pos.from_chinese_notation("馬二进三").is_ok());
        // 繁体棋子字 + 标准动作字
        let mv = pos.from_chinese_notation("馬二进三").unwrap();
        assert_eq!(pos.to_iccs_string(mv), "h0g2");
    }

    /// 往返：从初始局面枚举全部 44 步，逐个生成记谱再解析回来。
    #[test]
    fn roundtrip_from_startpos() {
        let mut pos = Position::startpos();
        let moves = pos.legal_moves();
        assert_eq!(moves.len(), 44);
        for mv in moves {
            let text = pos.to_chinese_notation(mv).expect("生成记谱失败");
            let parsed = pos
                .from_chinese_notation(&text)
                .unwrap_or_else(|e| panic!("解析 {text} 失败: {e}"));
            assert_eq!(parsed, mv, "记谱 {text} 往返后着法不一致");
        }
    }

    /// 解析：从初始局面按记谱走若干步经典开局。
    ///
    /// > 注意黑方的路数方向：黑方从自己的右侧（即棋图的 `a` 列）数 1 路，
    /// > 故 `a9` 的黑车是「车1」、`i9` 的黑车是「车9」。这与很多人的直觉相反。
    #[test]
    fn parse_classic_opening_moves() {
        let mut pos = Position::startpos();
        let expected = [
            ("炮二平五", "h2e2"),
            ("炮8平5", "h7e7"),
            ("马二进三", "h0g2"),
            ("马8进7", "h9g7"),
            ("车一平二", "i0h0"),
            // 黑方 a9 车是「车1」，但 b9 被己方马占住，故只有 i9 车能动
            ("车9平8", "i9h9"),
            ("兵七进一", "c3c4"),
            ("卒7进1", "g6g5"),
        ];
        for (text, iccs) in expected {
            let mv = pos
                .from_chinese_notation(text)
                .unwrap_or_else(|e| panic!("解析 {text} 失败: {e}"));
            assert_eq!(pos.to_iccs_string(mv), iccs, "记谱 {text} 解析出的着法不对");
            // 走完再验证一遍生成
            assert_eq!(pos.to_chinese_notation(mv).unwrap(), text);
            pos.make_move(mv).expect("着法应合法");
        }
    }

    /// 错误路径。
    #[test]
    fn parse_errors() {
        let mut pos = Position::startpos();

        assert_eq!(pos.from_chinese_notation(""), Err(NotationError::Empty));

        // 「十」既不是汉字数字也不在 1..9，故在「取值」阶段就失败
        assert_eq!(
            pos.from_chinese_notation("炮二平十"),
            Err(NotationError::MissingValue)
        );

        // 「进」后跟了不可能的值 7（兵最多进 1 格）
        assert!(matches!(
            pos.from_chinese_notation("兵三进七"),
            Err(NotationError::NoMatch)
        ));

        // 五路上没有红炮
        assert!(matches!(
            pos.from_chinese_notation("炮五平五"),
            Err(NotationError::NoMatch)
        ));

        // 缺少动作字
        assert_eq!(
            pos.from_chinese_notation("炮二平"),
            Err(NotationError::MissingValue)
        );

        // 无法识别的棋子字
        assert_eq!(
            pos.from_chinese_notation("龍二进三"),
            Err(NotationError::UnknownPieceChar('龍'))
        );
    }

    /// 红黑字形不同：轮黑方走时写「帅」应被判为非法记谱，而不是误匹配到黑将。
    #[test]
    fn wrong_side_glyph_is_rejected() {
        let mut pos = Position::startpos();
        // 先走一步红着，把走子方交给黑方
        let mv = pos.from_chinese_notation("炮二平五").unwrap();
        pos.make_move(mv).unwrap();
        assert_eq!(pos.side_to_move(), Color::Black);

        assert_eq!(
            pos.from_chinese_notation("帅五进一"),
            Err(NotationError::UnknownPieceChar('帅')),
            "黑方走子时不应接受红方的「帅」字"
        );
        // 同一个着法用黑方字形写就应当能解析
        assert!(pos.from_chinese_notation("将5进1").is_ok());
    }
}
