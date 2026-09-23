//! LLM 增强接口与幻觉抑制。
//!
//! # 三层防线（[docs/05 §6.3](../../../docs/05-战法讲解引擎设计.md)）
//!
//! | 防线 | 手段 | 处理 |
//! |---|---|---|
//! | ① 输入约束 | 只喂结构化事实，不喂原始棋盘 | 从源头杜绝「看错棋」 |
//! | ② 输出事实校验 | 检查输出是否出现输入中**没有**的坐标/着法/战法名 | 不过 → 丢弃 |
//! | ③ 长度与格式校验 | 字数区间、无 Markdown、无异常字符 | 不过 → 丢弃 |
//!
//! **丢弃后的行为**：保留本地模板讲解，`llm_status = Failed`，用户看到的仍是
//! 完整正确的讲解。**LLM 永不在关键路径上** —— 它失败的唯一后果是少一段润色。
//!
//! # 本模块不含任何 IO
//!
//! trait 只定义契约；实际的 HTTP 实现放在 `xq-server` / `xq-client`。
//! 客户端的实现必须**经由自己的服务端代理**，不得内置服务商 API Key。

use std::collections::HashSet;

use crate::note::CoachNote;

/// 一次 LLM 调用请求。
#[derive(Clone, Debug)]
pub struct LlmRequest {
    /// 系统提示词。
    pub system_prompt: String,
    /// 用户提示词（只含结构化事实）。
    pub user_prompt: String,
    /// 输出上限（token）。
    pub max_tokens: u32,
    /// 采样温度。润色任务宜低。
    pub temperature: f32,
    /// 硬超时（毫秒），由实现方保证。
    pub timeout_ms: u64,
}

/// 一次 LLM 调用的响应。
#[derive(Clone, Debug)]
pub struct LlmResponse {
    /// 生成的文本。
    pub text: String,
    /// token 用量，用于预算统计。
    pub usage: Option<TokenUsage>,
}

/// token 用量。
#[derive(Clone, Copy, Debug, Default)]
pub struct TokenUsage {
    /// 输入 token。
    pub prompt_tokens: u32,
    /// 输出 token。
    pub completion_tokens: u32,
}

/// LLM 调用错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LlmError {
    /// 超时。
    Timeout,
    /// 传输层错误（网络、HTTP 状态等）。
    Transport(String),
    /// 被上游拒绝（限流、内容策略）。
    Rejected(String),
}

impl core::fmt::Display for LlmError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LlmError::Timeout => write!(f, "LLM 调用超时"),
            LlmError::Transport(e) => write!(f, "LLM 传输错误：{e}"),
            LlmError::Rejected(e) => write!(f, "LLM 拒绝请求：{e}"),
        }
    }
}

impl std::error::Error for LlmError {}

/// LLM 客户端契约。由应用层实现（`xq-server` / `xq-client`）。
pub trait LlmClient: Send + Sync {
    /// 发起一次补全调用。实现方须自行保证超时。
    fn complete(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError>;
}

/// 一个永远失败的客户端 —— 用于「未配置 LLM」的默认值，让调用方不必处理 `None`。
#[derive(Debug, Default, Clone, Copy)]
pub struct NoLlm;

impl LlmClient for NoLlm {
    fn complete(&self, _request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(LlmError::Rejected("未配置 LLM 客户端".to_string()))
    }
}

// ---------------------------------------------------------------- 事实集合

/// 允许在 LLM 输出中出现的实体集合。
///
/// 由本地讲解结果构造 —— 这正是「只喂结构化事实」的落地方式：
/// 允许出现的实体就是喂给 LLM 的那些。
#[derive(Clone, Debug, Default)]
pub struct FactSet {
    allowed: HashSet<String>,
}

impl FactSet {
    /// 从一条本地讲解记录里提取允许出现的实体。
    ///
    /// 收进来的都是**已知为真**的东西：本步记谱、走子方、棋子名、被吃子、
    /// 最优着法、战术名、开局名、评分数字。
    pub fn from_note(note: &CoachNote) -> Self {
        let mut allowed = HashSet::new();

        for text in [&note.notation, &note.mv_iccs, &note.headline, &note.detail] {
            Self::collect_entities(text, &mut allowed);
        }
        for tag in &note.tactics {
            allowed.insert(tag.name.clone());
            allowed.insert(tag.id.clone());
        }
        if let Some(opening) = &note.opening_name {
            allowed.insert(opening.clone());
        }
        if let Some(suggestion) = &note.suggestion {
            Self::collect_entities(suggestion, &mut allowed);
        }
        for line in &note.pv {
            Self::collect_entities(line, &mut allowed);
        }
        // 常见词汇：棋子名与量词，模型用它们组句不算幻觉
        for word in [
            "帅", "将", "仕", "士", "相", "象", "马", "车", "炮", "兵", "卒", "红", "黑", "方",
            "一", "二", "三", "四", "五", "六", "七", "八", "九", "1", "2", "3", "4", "5", "6",
            "7", "8", "9",
        ] {
            allowed.insert(word.to_string());
        }

        Self { allowed }
    }

    /// 追加一个允许实体。
    pub fn allow(&mut self, entity: impl Into<String>) {
        self.allowed.insert(entity.into());
    }

    /// 某实体是否被允许。
    pub fn contains(&self, entity: &str) -> bool {
        self.allowed.contains(entity)
    }

    /// 允许实体的数量。
    pub fn len(&self) -> usize {
        self.allowed.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.allowed.is_empty()
    }

    /// 从文本里提取疑似实体：ICCS 坐标、中文记谱片段。
    fn collect_entities(text: &str, out: &mut HashSet<String>) {
        for entity in extract_entities(text) {
            out.insert(entity);
        }
    }
}

/// 输出校验失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RejectReason {
    /// 出现了输入中不存在的实体（幻觉）。
    Hallucination(Vec<String>),
    /// 长度不在允许区间。
    Length(usize),
    /// 含 Markdown 标记。
    Markdown,
    /// 含异常字符。
    WeirdChars,
    /// 输出为空。
    Empty,
}

impl RejectReason {
    /// 面向日志的说明。
    pub fn describe(&self) -> String {
        match self {
            RejectReason::Hallucination(items) => {
                format!("出现未提供的实体：{}", items.join("、"))
            }
            RejectReason::Length(n) => format!("长度 {n} 字，超出允许区间"),
            RejectReason::Markdown => "含 Markdown 标记".to_string(),
            RejectReason::WeirdChars => "含异常字符".to_string(),
            RejectReason::Empty => "输出为空".to_string(),
        }
    }
}

/// 输出长度允许区间（字符数）。
pub const MIN_OUTPUT_CHARS: usize = 8;
/// 输出长度上限。
pub const MAX_OUTPUT_CHARS: usize = 200;

/// 校验 LLM 输出。
///
/// 三道防线里的 ② 与 ③。**任何一条不过即丢弃**，调用方应保留本地讲解。
pub fn validate_output(text: &str, facts: &FactSet) -> Result<(), RejectReason> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(RejectReason::Empty);
    }

    // ③ 长度
    let chars = trimmed.chars().count();
    if !(MIN_OUTPUT_CHARS..=MAX_OUTPUT_CHARS).contains(&chars) {
        return Err(RejectReason::Length(chars));
    }

    // ③ 格式：不得出现 Markdown 标记
    if trimmed.contains("**")
        || trimmed.contains("##")
        || trimmed.contains("```")
        || trimmed.contains("- ")
        || trimmed.contains("1. ")
    {
        return Err(RejectReason::Markdown);
    }

    // ③ 异常字符：换行符过多、控制字符
    if trimmed.matches('\n').count() > 2 || trimmed.chars().any(|c| c.is_control() && c != '\n') {
        return Err(RejectReason::WeirdChars);
    }

    // ② 事实校验
    let mut hallucinated: Vec<String> = Vec::new();
    for entity in extract_entities(trimmed) {
        if !facts.contains(&entity) && !hallucinated.contains(&entity) {
            hallucinated.push(entity);
        }
    }
    if !hallucinated.is_empty() {
        return Err(RejectReason::Hallucination(hallucinated));
    }

    Ok(())
}

/// 从文本里提取「疑似实体」。
///
/// 只提取两类**高度可识别**的东西，宁可漏检也不误报：
///
/// 1. **ICCS 坐标**：`[a-i][0-9]`，如 `h2`；
/// 2. **中文记谱**：`[棋子字][数字][进退平][数字]`，如 `炮二平五`。
///
/// 不做「任意中文词」的抽取 —— 那会把正常词汇全判成幻觉。
fn extract_entities(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();

    for i in 0..chars.len() {
        // ---- ICCS 坐标 ----
        if ('a'..='i').contains(&chars[i])
            && let Some(next) = chars.get(i + 1)
            && next.is_ascii_digit()
        {
            out.push(format!("{}{}", chars[i], next));
            continue;
        }

        // ---- 中文记谱：棋子字 + 数字 + 动作 + 数字 ----
        if is_piece_char(chars[i])
            && let (Some(n1), Some(action), Some(n2)) =
                (chars.get(i + 1), chars.get(i + 2), chars.get(i + 3))
            && is_numeral(*n1)
            && matches!(action, '进' | '退' | '平')
            && is_numeral(*n2)
        {
            out.push(chars[i..i + 4].iter().collect());
        }
    }

    out
}

fn is_piece_char(c: char) -> bool {
    matches!(
        c,
        '帅' | '将' | '仕' | '士' | '相' | '象' | '马' | '车' | '炮' | '兵' | '卒'
    )
}

fn is_numeral(c: char) -> bool {
    matches!(
        c,
        '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '1'..='9'
    )
}

// ---------------------------------------------------------------- 提示词

/// 系统提示词。
///
/// 核心原则：**LLM 只做语言润色，不做事实推理**。
pub const SYSTEM_PROMPT: &str = "\
你是一位中国象棋教练。你的任务是把给定的局面分析结果改写成自然、易懂的中文点评。

严格约束：
1. 只能使用【分析结果】中提供的事实，不得添加任何未提供的棋子位置、着法或战法名称。
2. 不得改变评价等级与评分结论。
3. 不得编造变化推演。若【分析结果】中未给出后续变化，就不要推演后续。
4. 输出 2~3 句中文，不要使用列表、标题、Markdown 标记。
5. 不要重复「这步棋」等无信息量的开头。";

/// 构造用户提示词。
///
/// **只喂结构化事实，不喂原始棋盘** —— 这是第一道防线。
pub fn build_user_prompt(note: &CoachNote, extra_facts: &[&str]) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("【局面】");
    out.push_str(match note.side {
        xq_core::Color::Red => "红方走子\n",
        xq_core::Color::Black => "黑方走子\n",
    });

    out.push_str(&format!("【着法】{}\n", note.notation));
    out.push_str(&format!(
        "【评价】{}（分差 {} 厘兵）\n",
        note.level.label(),
        note.score_loss
    ));

    if !note.tactics.is_empty() {
        let names: Vec<&str> = note.tactics.iter().map(|t| t.name.as_str()).collect();
        out.push_str(&format!("【战术】{}\n", names.join("、")));
    }
    if let Some(opening) = &note.opening_name {
        out.push_str(&format!("【开局】{opening}\n"));
    }
    if let Some(suggestion) = &note.suggestion {
        out.push_str(&format!("【引擎分析】{suggestion}\n"));
    }
    for fact in extra_facts {
        out.push_str(&format!("【补充事实】{fact}\n"));
    }

    out.push_str("\n请改写为教练点评。");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::{CoachNote, LlmStatus, MoveLevel, NoteSource, TacticCategory, TacticTag};
    use xq_core::Color;

    fn sample_note() -> CoachNote {
        CoachNote {
            ply: 1,
            side: Color::Red,
            mv_iccs: "h2e2".into(),
            notation: "炮二平五".into(),
            level: MoveLevel::Best,
            score_loss: 0,
            score_before: 20,
            score_after: 20,
            tactics: vec![TacticTag {
                id: "opening_central_cannon".into(),
                name: "中炮".into(),
                category: TacticCategory::Opening,
                confidence: 0.9,
            }],
            headline: "红方 炮二平五。".into(),
            detail: "炮发起了对中路的控制。".into(),
            suggestion: None,
            source: NoteSource::Local,
            llm_status: LlmStatus::None,
            pv: vec!["炮二平五".into()],
            opening_name: Some("中炮对屏风马".into()),
            template_id: "tpl.opening_central_cannon".into(),
            used_fallback: false,
        }
    }

    #[test]
    fn fact_set_includes_known_entities() {
        let facts = FactSet::from_note(&sample_note());
        assert!(facts.contains("炮二平五"));
        assert!(facts.contains("中炮"));
        assert!(facts.contains("中炮对屏风马"));
        assert!(facts.len() > 5);
    }

    /// 防线 ②：输出里出现输入中没有的着法 → 必须被判为幻觉。
    #[test]
    fn detects_hallucinated_move() {
        let facts = FactSet::from_note(&sample_note());
        // 「车一平二」从未出现在输入里
        let bad = "红方走了炮二平五，随后可以车一平二展开攻势，形成压制。";
        match validate_output(bad, &facts) {
            Err(RejectReason::Hallucination(items)) => {
                assert!(items.iter().any(|i| i == "车一平二"), "应抓到车一平二");
            }
            other => panic!("应判为幻觉，实际 {other:?}"),
        }
    }

    /// 输出里出现输入中没有的坐标 → 幻觉。
    #[test]
    fn detects_hallucinated_coordinate() {
        let facts = FactSet::from_note(&sample_note());
        let bad = "炮二平五之后红方控制 h5 一带的重要位置，局面主动。";
        match validate_output(bad, &facts) {
            Err(RejectReason::Hallucination(items)) => {
                assert!(items.iter().any(|i| i == "h5"));
            }
            other => panic!("应判为幻觉，实际 {other:?}"),
        }
    }

    /// 正常的润色输出应通过校验。
    #[test]
    fn accepts_faithful_rewrite() {
        let facts = FactSet::from_note(&sample_note());
        let good =
            "红方以炮二平五直取中路，这是当前最优的着法。中炮就位后，红方在中路形成了持续压力。";
        assert!(validate_output(good, &facts).is_ok());
    }

    /// 防线 ③：Markdown 标记必须被拦。
    #[test]
    fn rejects_markdown() {
        let facts = FactSet::from_note(&sample_note());
        let bad = "**红方炮二平五**，控制中路。";
        assert_eq!(validate_output(bad, &facts), Err(RejectReason::Markdown));
    }

    /// 防线 ③：过短 / 过长都要拦。
    #[test]
    fn rejects_length_out_of_range() {
        let facts = FactSet::from_note(&sample_note());
        assert_eq!(validate_output("好", &facts), Err(RejectReason::Length(1)));

        let long: String = "红方炮二平五。".repeat(40);
        assert!(matches!(
            validate_output(&long, &facts),
            Err(RejectReason::Length(_))
        ));
    }

    #[test]
    fn rejects_empty() {
        let facts = FactSet::from_note(&sample_note());
        assert_eq!(validate_output("   ", &facts), Err(RejectReason::Empty));
    }

    /// 普通词汇不应被误判成实体。
    #[test]
    fn plain_chinese_is_not_flagged() {
        let facts = FactSet::from_note(&sample_note());
        let text = "这一步走得很稳，把主动权握在手里，对方需要小心应对接下来的变化。";
        assert!(
            validate_output(text, &facts).is_ok(),
            "正常中文不应被判为幻觉"
        );
    }

    #[test]
    fn entity_extraction_finds_both_kinds() {
        let found = extract_entities("炮二平五之后 h2 的车可以动，马8进7也是选择。");
        assert!(found.contains(&"炮二平五".to_string()));
        assert!(found.contains(&"h2".to_string()));
        assert!(found.contains(&"马8进7".to_string()));
    }

    #[test]
    fn no_llm_client_always_fails_gracefully() {
        let client = NoLlm;
        let request = LlmRequest {
            system_prompt: SYSTEM_PROMPT.to_string(),
            user_prompt: "x".to_string(),
            max_tokens: 256,
            temperature: 0.3,
            timeout_ms: 3000,
        };
        assert!(matches!(
            client.complete(&request),
            Err(LlmError::Rejected(_))
        ));
    }

    #[test]
    fn prompt_contains_only_structured_facts() {
        let note = sample_note();
        let prompt = build_user_prompt(&note, &["该着法使红方在中路形成持续压力"]);
        assert!(prompt.contains("炮二平五"));
        assert!(prompt.contains("最佳着法"));
        assert!(prompt.contains("中炮对屏风马"));
        // 绝不把原始棋盘喂进去
        assert!(!prompt.contains("rnbakabnr"), "提示词里不应出现原始 FEN");
    }
}
