//! 命令行演练场 —— 直接拿 `xq-core` 下棋。
//!
//! ```bash
//! cargo run --example xq              # 中文棋子字形（默认）
//! cargo run --example xq -- --ascii   # 字母棋子字形（终端显示中文乱码时用）
//! ```
//!
//! 这是一个**开发工具**，不是产品界面。用途是：
//!
//! - 手工验证规则（走一步立刻看到合法性校验、将军判定、记谱输出）；
//! - 调试具体局面（`fen <字符串>` 直接载入）；
//! - 生成记谱文本（`hist` 输出整局记谱）。
//!
//! > 本文件位于 `examples/`，属于开发工具，可以使用 `std::io`。
//! > ADR-005 的零 IO 约束只作用于 `crates/*/src/`，CI 断言脚本也仅扫描该目录。

use std::io::{self, BufRead, Write};

use xq_core::piece::{color_of, kind_of};
use xq_core::square::{COLS, ROWS, from_iccs, index, route_number, to_iccs};
use xq_core::{Color, EMPTY, GameStatus, Move, PieceKind, Position};

/// 棋盘行前缀 `" {row:>2} "` 的显示宽度（1 + 2 + 1）。
const PREFIX: usize = 4;
/// 每格可用宽度（两侧各留一格空白，中间放棋子）。
const INNER: usize = 2;
/// 每格连边框占用的显示宽度：INNER + 2（两侧空格）+ 1（右边框）。
const STRIDE: usize = INNER + 3;
/// 整行显示宽度：前缀 + 左边框 + 9 格。
const LINE_WIDTH: usize = PREFIX + 1 + COLS as usize * STRIDE;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut ascii = args.iter().any(|a| a == "--ascii");

    let mut pos = Position::startpos();
    let mut log: Vec<String> = Vec::new();
    let mut last_move: Option<Move> = None;
    let mut last_notation = String::new();

    print_welcome(ascii);

    loop {
        let status = pos.status();
        render(&pos, status, ascii, last_move, &last_notation);

        let prompt = if status.is_over() {
            "对局已结束 · 输入 undo 悔棋或 new 重开 > ".to_string()
        } else {
            format!("{}方 > ", pos.side_to_move().name_zh())
        };

        let Some(line) = read_line(&prompt) else {
            println!();
            break;
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let mut state = UiState {
            ascii,
            log: &mut log,
            last_move: &mut last_move,
            last_notation: &mut last_notation,
        };

        match handle_input(&line, &mut pos, &mut state) {
            Outcome::Continue => {}
            Outcome::Quit => break,
            Outcome::Message(msg) => println!("{msg}"),
        }
        ascii = state.ascii;
    }

    println!("再见。");
}

/// 跨命令可变状态的打包，避免函数签名过长。
struct UiState<'a> {
    ascii: bool,
    log: &'a mut Vec<String>,
    last_move: &'a mut Option<Move>,
    last_notation: &'a mut String,
}

enum Outcome {
    Continue,
    Quit,
    Message(String),
}

// ---------------------------------------------------------------- 输入处理

fn handle_input(line: &str, pos: &mut Position, st: &mut UiState<'_>) -> Outcome {
    let (cmd, arg) = split_command(line);

    match cmd {
        "quit" | "q" | "exit" => return Outcome::Quit,
        "help" | "?" => return Outcome::Message(HELP.to_string()),

        "new" => {
            *pos = Position::startpos();
            st.log.clear();
            *st.last_move = None;
            st.last_notation.clear();
            return Outcome::Message("已重置为初始局面。".to_string());
        }

        "ascii" => {
            st.ascii = true;
            return Outcome::Message("已切换为字母字形（K A B N R C P / 小写为黑）。".to_string());
        }
        "cn" | "chinese" => {
            st.ascii = false;
            return Outcome::Message("已切换为中文字形。".to_string());
        }

        "board" | "b" => return Outcome::Continue,

        "fen" => {
            if arg.is_empty() {
                return Outcome::Message(format!("当前 FEN：\n{}", pos.to_fen()));
            }
            return match Position::from_fen(arg) {
                Ok(p) => {
                    let loaded = p.to_fen();
                    *pos = p;
                    st.log.clear();
                    *st.last_move = None;
                    st.last_notation.clear();
                    Outcome::Message(format!("已载入局面。\nFEN：{loaded}"))
                }
                Err(e) => Outcome::Message(format!("FEN 解析失败：{e}")),
            };
        }

        "status" | "s" => {
            let mut msg = describe_status(pos.status(), pos);
            if let Some(verdict) = xq_core::repetition::adjudicate(pos) {
                msg.push_str(&format!("\n重复局面裁决：{}", verdict.description()));
            }
            return Outcome::Message(msg);
        }

        "legal" | "l" => return Outcome::Message(list_all_legal(pos)),

        "hist" | "history" => {
            if st.log.is_empty() {
                return Outcome::Message("尚无着法。".to_string());
            }
            let mut out = String::from("着法记录：\n");
            for (i, text) in st.log.iter().enumerate() {
                let side = if i % 2 == 0 { "红" } else { "黑" };
                out.push_str(&format!("  {:>3}. {}{}\n", i + 1, side, text));
            }
            return Outcome::Message(out.trim_end().to_string());
        }

        "undo" | "u" => {
            return match pos.unmake_move() {
                Some(_) => {
                    st.log.pop();
                    *st.last_move = None;
                    st.last_notation.clear();
                    Outcome::Message("已悔一步。".to_string())
                }
                None => Outcome::Message("已经在初始局面，无法再悔。".to_string()),
            };
        }

        "hint" | "h" => return Outcome::Message(hint(pos, arg)),

        _ => {}
    }

    // 不是命令，按着法解析
    if pos.status().is_over() {
        return Outcome::Message("对局已结束，请先 undo 或 new。".to_string());
    }

    let mv = match parse_move(pos, line) {
        Ok(mv) => mv,
        Err(e) => {
            return Outcome::Message(format!("看不懂这个输入：{e}\n（输入 help 查看用法）"));
        }
    };

    let notation = pos
        .to_chinese_notation(mv)
        .unwrap_or_else(|_| "（记谱生成失败）".to_string());
    let iccs = pos.to_iccs_string(mv);
    let captured = pos.piece_at(mv.to());

    if let Err(e) = pos.make_move(mv) {
        return Outcome::Message(format!("着法非法：{e}"));
    }

    let mut msg = format!("  {notation}   ({iccs})");
    if captured != EMPTY {
        let kind = kind_of(captured).expect("被吃棋子必有种类");
        let color = color_of(captured).expect("被吃棋子必有颜色");
        msg.push_str(&format!(
            "   吃掉{}方{}",
            color.name_zh(),
            kind.name_zh(color)
        ));
    }
    if let Some(rec) = pos.move_stack().last()
        && rec.gave_check
    {
        msg.push_str("   【将军】");
    }
    let status = pos.status();
    if status.is_over() {
        msg.push('\n');
        msg.push_str(&describe_status(status, pos));
    }

    st.log.push(notation.clone());
    *st.last_move = Some(mv);
    *st.last_notation = notation;
    Outcome::Message(msg)
}

/// 把一行输入拆成「命令 + 剩余参数」。
fn split_command(line: &str) -> (&str, &str) {
    match line.find(char::is_whitespace) {
        Some(i) => (&line[..i], line[i..].trim()),
        None => (line, ""),
    }
}

/// 解析着法输入：先试 ICCS（含带分隔符的形式），再试中文记谱。
fn parse_move(pos: &mut Position, line: &str) -> Result<Move, String> {
    let compact: String = line
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '>')
        .collect();

    if compact.len() == 4
        && compact.is_ascii()
        && let (Some(from), Some(to)) = (from_iccs(&compact[0..2]), from_iccs(&compact[2..4]))
    {
        return pos
            .legal_move_to(from, to)
            .ok_or_else(|| format!("{compact} 不是当前局面下的合法着法"));
    }

    pos.from_chinese_notation(line).map_err(|e| e.to_string())
}

/// 提示：给出某格棋子的全部合法落点；不给格子则列出全部合法着法。
fn hint(pos: &mut Position, arg: &str) -> String {
    if arg.is_empty() {
        return list_all_legal(pos);
    }
    let Some(from) = from_iccs(arg) else {
        return format!("{arg} 不是合法坐标（应形如 h2，列用 a-i、行用 0-9）");
    };
    let piece = pos.piece_at(from);
    if piece == EMPTY {
        return format!("{arg} 是空格。");
    }
    let kind = kind_of(piece).expect("非空棋子必有种类");
    let color = color_of(piece).expect("非空棋子必有颜色");

    let legal = pos.legal_moves();
    let mut items: Vec<String> = Vec::new();
    for mv in legal.iter().filter(|m| m.from() == from) {
        let notation = pos
            .to_chinese_notation(*mv)
            .unwrap_or_else(|_| "?".to_string());
        let mark = if pos.piece_at(mv.to()) == EMPTY {
            ""
        } else {
            "吃"
        };
        items.push(format!("{notation}{mark}→{}", to_iccs(mv.to())));
    }

    if items.is_empty() {
        return format!(
            "{}方{}在 {} 处没有合法着法（被钉住、被蹩住，或走哪都会被将）。",
            color.name_zh(),
            kind.name_zh(color),
            arg
        );
    }
    format!(
        "{}方{}（{}）可走 {} 步：\n  {}",
        color.name_zh(),
        kind.name_zh(color),
        arg,
        items.len(),
        items.join("   ")
    )
}

fn list_all_legal(pos: &mut Position) -> String {
    let legal = pos.legal_moves();
    let mut items: Vec<String> = legal
        .iter()
        .map(|mv| {
            let notation = pos
                .to_chinese_notation(*mv)
                .unwrap_or_else(|_| "?".to_string());
            format!("{notation}({})", pos.to_iccs_string(*mv))
        })
        .collect();
    items.sort();
    format!(
        "{}方共 {} 种合法着法：\n  {}",
        pos.side_to_move().name_zh(),
        items.len(),
        items.join("  ")
    )
}

fn describe_status(status: GameStatus, pos: &Position) -> String {
    match status {
        GameStatus::Ongoing => "进行中。".to_string(),
        GameStatus::Check { side } => format!("{}方被将军，必须应将。", side.name_zh()),
        GameStatus::Checkmate { loser } => format!(
            "将死！{}方负，{}方胜。",
            loser.name_zh(),
            loser.opponent().name_zh()
        ),
        GameStatus::Stalemate { loser } => format!(
            "困毙！{}方无着可走（且未被将军）。注意：中国象棋判困毙方**负**，与国际象棋判和不同。",
            loser.name_zh()
        ),
        GameStatus::Draw { reason } => {
            let mut s = format!("和棋 · {}", reason.description());
            if pos.is_insufficient_material() {
                s.push_str("（当前双方均无进攻子力）");
            }
            s
        }
    }
}

// ---------------------------------------------------------------- 渲染

fn read_line(prompt: &str) -> Option<String> {
    print!("{prompt}");
    io::stdout().flush().ok()?;
    let mut buf = String::new();
    match io::stdin().lock().read_line(&mut buf) {
        Ok(0) => None,
        Ok(_) => Some(buf),
        Err(_) => None,
    }
}

fn render(
    pos: &Position,
    status: GameStatus,
    ascii: bool,
    last_move: Option<Move>,
    last_notation: &str,
) {
    println!();
    if last_notation.is_empty() {
        println!("  （尚未走子）");
    } else {
        println!("  上一着：{last_notation}");
    }
    println!();

    println!("{}", ruler(&['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i']));
    println!("{}", border('┌', '┬', '┐'));

    for row in (0..ROWS).rev() {
        let mut line = format!(" {row:>2} │");
        for col in 0..COLS {
            let piece = pos.piece_at(index(col, row));
            line.push(' ');
            line.push_str(&pad_display(&glyph(piece, ascii), INNER));
            line.push_str(" │");
        }
        if row == 5 {
            line.push_str("  ← 楚河汉界");
        }
        println!("{line}");

        if row > 0 {
            println!("{}", border('├', '┼', '┤'));
        }
    }

    println!("{}", border('└', '┴', '┘'));

    let red: Vec<char> = (0..COLS)
        .map(|c| digit(route_number(Color::Red, c)))
        .collect();
    let black: Vec<char> = (0..COLS)
        .map(|c| digit(route_number(Color::Black, c)))
        .collect();
    println!("{}   ← 红方路数（红方从自己的右侧数起）", ruler(&red));
    println!("{}   ← 黑方路数（黑方从自己的右侧数起）", ruler(&black));

    println!();
    println!("  {}", describe_status(status, pos));

    if let Some(mv) = last_move {
        println!(
            "  · 半回合计数 {} · 第 {} 回合 · 上一着坐标 {} → {}",
            pos.halfmove_clock(),
            pos.fullmove_number(),
            to_iccs(mv.from()),
            to_iccs(mv.to())
        );
    } else {
        println!(
            "  · 半回合计数 {} · 第 {} 回合",
            pos.halfmove_clock(),
            pos.fullmove_number()
        );
    }
}

fn border(left: char, mid: char, right: char) -> String {
    let mut s = String::new();
    s.push_str(&" ".repeat(PREFIX));
    s.push(left);
    for i in 0..COLS {
        s.push_str(&"─".repeat(INNER + 2));
        if i + 1 < COLS {
            s.push(mid);
        }
    }
    s.push(right);
    s
}

/// 生成与棋盘列对齐的标注行（列字母 / 路数）。
fn ruler(labels: &[char]) -> String {
    let mut buf = vec![' '; LINE_WIDTH];
    for (i, ch) in labels.iter().enumerate() {
        // 第 i 格的棋子内容占显示列 [PREFIX+2 + i*STRIDE, +1]
        let pos = PREFIX + 2 + i * STRIDE;
        if pos < LINE_WIDTH {
            buf[pos] = *ch;
        }
    }
    buf.into_iter().collect()
}

fn digit(n: u8) -> char {
    char::from_digit(n as u32, 10).expect("路数必为 1..=9")
}

fn glyph(piece: u8, ascii: bool) -> String {
    if piece == EMPTY {
        return "·".to_string();
    }
    let kind = kind_of(piece).expect("非空棋子必有种类");
    let color = color_of(piece).expect("非空棋子必有颜色");
    if ascii {
        let c = match kind {
            PieceKind::King => 'K',
            PieceKind::Advisor => 'A',
            PieceKind::Elephant => 'B',
            PieceKind::Horse => 'N',
            PieceKind::Chariot => 'R',
            PieceKind::Cannon => 'C',
            PieceKind::Pawn => 'P',
        };
        let c = if color == Color::Red {
            c
        } else {
            c.to_ascii_lowercase()
        };
        c.to_string()
    } else {
        kind.name_zh(color).to_string()
    }
}

/// 按**显示宽度**右侧补空格（CJK 字符在终端里占 2 列）。
fn pad_display(text: &str, width: usize) -> String {
    let mut out = String::from(text);
    let mut w = text.chars().map(char_width).sum::<usize>();
    while w < width {
        out.push(' ');
        w += 1;
    }
    out
}

fn char_width(ch: char) -> usize {
    let cp = ch as u32;
    let wide = (0x2E80..=0x9FFF).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFE30..=0xFE4F).contains(&cp)
        || (0xFF00..=0xFF60).contains(&cp)
        || (0xFFE0..=0xFFE6).contains(&cp);
    if wide { 2 } else { 1 }
}

// ---------------------------------------------------------------- 其他

fn print_welcome(ascii: bool) {
    println!(
        "弈道 · 规则内核演练场（xq-core {}）",
        env!("CARGO_PKG_VERSION")
    );
    println!("────────────────────────────────────────────────────────");
    println!("  红方用汉字数字、黑方用阿拉伯数字记谱，例如：");
    println!("    炮二平五    红炮从二路平到五路（当头炮）");
    println!("    马8进7      黑马从 8 路进到 7 路（标准应着）");
    println!("  也可以直接输坐标： h2e2");
    println!("  输入 help 查看全部命令。");
    if ascii {
        println!("  [当前为字母字形]");
    }
    println!();
}

const HELP: &str = "\
用法
────────────────────────────────────────────────────────
下棋
  炮二平五          中文记谱（红方汉字数字、黑方阿拉伯数字）
  h2e2              ICCS 坐标
  h2 e2 / h2-e2     同上，可带分隔符

查看
  board / b         重画棋盘（每次走子后本就会重画）
  legal / l         列出当前全部合法着法
  hint h2  / h h2   某格棋子能走到哪里
  status / s        当前状态（将军 / 将死 / 困毙 / 和棋）
  hist              整局着法记录（中文记谱）
  fen               打印当前 FEN
  fen <字符串>      载入指定局面

操作
  undo / u          悔一步
  new               重开
  ascii / cn        切换字母 / 中文棋子字形
  help / ?          显示本帮助
  quit / q          退出

试试这些
  炮二平五          当头炮
  马8进7            黑方标准应着
  hint h2           看红炮能去哪
  legal             看现在有多少种走法
  车一平二          出车
";
