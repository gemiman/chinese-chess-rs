//! 讲解输出的数据结构 —— 对客户端与前端的主要契约。

use serde::{Deserialize, Serialize};
use xq_core::Color;

/// 评价等级。
///
/// 定级**完全基于客观的评分损失**（`score_best - score_played`），不引入主观判断。
/// 阈值见 [docs/05 §4.2](../../../docs/05-战法讲解引擎设计.md)，
/// 且明确声明**必须回测校准**——当前阈值是设计初始值。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum MoveLevel {
    /// 优：分差 ≤ 10 厘兵。
    Best,
    /// 良：10 < 分差 ≤ 50。
    Good,
    /// 疑：50 < 分差 ≤ 150。
    Dubious,
    /// 劣：150 < 分差 ≤ 400。
    Blunder,
    /// 漏：> 400，或错失必胜。
    Missed,
}

impl MoveLevel {
    /// 全部等级，由优到劣。
    pub const ALL: [MoveLevel; 5] = [
        MoveLevel::Best,
        MoveLevel::Good,
        MoveLevel::Dubious,
        MoveLevel::Blunder,
        MoveLevel::Missed,
    ];

    /// UI 文案。
    ///
    /// ⚠️ 按 [docs/05 §4.3](../../../docs/05-战法讲解引擎设计.md) 的强制约束，
    /// 等级**不得仅用颜色传达** —— 必须同时有文字标签或图标。
    pub const fn label(self) -> &'static str {
        match self {
            MoveLevel::Best => "最佳着法",
            MoveLevel::Good => "不错",
            MoveLevel::Dubious => "稍有问题",
            MoveLevel::Blunder => "明显失误",
            MoveLevel::Missed => "严重漏着",
        }
    }

    /// 用于色觉障碍区分的图标字形（形状不同，而非仅颜色不同）。
    pub const fn glyph(self) -> &'static str {
        match self {
            MoveLevel::Best => "★",
            MoveLevel::Good => "✓",
            MoveLevel::Dubious => "?",
            MoveLevel::Blunder => "✕",
            MoveLevel::Missed => "‼",
        }
    }

    /// 颜色令牌名（对应 `assets/tokens/design-tokens.json` 的 `coach.*`）。
    pub const fn token(self) -> &'static str {
        match self {
            MoveLevel::Best => "coach.best",
            MoveLevel::Good => "coach.good",
            MoveLevel::Dubious => "coach.dubious",
            MoveLevel::Blunder => "coach.blunder",
            MoveLevel::Missed => "coach.missed",
        }
    }

    /// 通用模板 id（降级链第 3 级）。
    pub const fn generic_template_id(self) -> &'static str {
        match self {
            MoveLevel::Best => "tpl.generic.best",
            MoveLevel::Good => "tpl.generic.good",
            MoveLevel::Dubious => "tpl.generic.dubious",
            MoveLevel::Blunder => "tpl.generic.blunder",
            MoveLevel::Missed => "tpl.generic.missed",
        }
    }

    /// 是否为「需要重点解读」的等级 —— 也是 LLM 增强的触发条件。
    ///
    /// [docs/05 §6.4](../../../docs/05-战法讲解引擎设计.md)：默认只对这三档调用 LLM。
    /// 用户对「我走对了」不需要长篇解释，但对「我这步走坏了」极度需要说明 ——
    /// 这同时降低成本与提升价值。
    pub const fn needs_explanation(self) -> bool {
        matches!(
            self,
            MoveLevel::Dubious | MoveLevel::Blunder | MoveLevel::Missed
        )
    }
}

/// 讲解来源。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NoteSource {
    /// 本地模板渲染。
    Local,
    /// LLM 增强。
    Llm,
}

/// LLM 增强状态。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LlmStatus {
    /// 本档等级不需要增强，或未启用 LLM。
    None,
    /// 增强请求进行中。
    Pending,
    /// 已成功增强。
    Enhanced,
    /// 增强失败（超时 / 校验不过 / 出错）—— 用户侧不展示失败，保留本地讲解。
    Failed,
}

/// 战术类别。按**可判定性**分层，这是本模块最重要的设计取舍。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TacticCategory {
    /// 结构层（L1）：由规则精确判定，100% 确定。
    Structure,
    /// 关系层（L2）：基于攻击关系与交换价值，高可信。
    Relation,
    /// 棋形层（L3）：声明式棋形匹配，**需用真实棋谱校准**。
    Formation,
    /// 开局定式。
    Opening,
}

impl TacticCategory {
    /// 中文标签。
    pub const fn label(self) -> &'static str {
        match self {
            TacticCategory::Structure => "结构",
            TacticCategory::Relation => "关系",
            TacticCategory::Formation => "棋形",
            TacticCategory::Opening => "开局",
        }
    }

    /// 从知识库里的 category 字符串解析。
    pub fn parse(text: &str) -> Option<TacticCategory> {
        match text {
            "structure" => Some(TacticCategory::Structure),
            "relation" => Some(TacticCategory::Relation),
            "formation" => Some(TacticCategory::Formation),
            "opening" => Some(TacticCategory::Opening),
            _ => None,
        }
    }

    /// UI 是否应以**较强样式**展示。
    ///
    /// 棋形层是「需校准」的 —— 按 docs/05 §7 的要求，置信度不足时以灰色弱样式
    /// 展示，避免误导用户把未经校准的棋形名当成确定结论。
    pub const fn is_certain(self) -> bool {
        matches!(self, TacticCategory::Structure)
    }
}

/// 一条命中的战术标签。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TacticTag {
    /// 稳定标识，如 `fork`。
    pub id: String,
    /// 中文名，如「捉双」。
    pub name: String,
    /// 所属层。
    pub category: TacticCategory,
    /// 置信度 0.0~1.0。UI 按此分级展示。
    pub confidence: f32,
}

impl TacticTag {
    /// UI 是否应以弱样式展示（仅棋形层可能如此）。
    pub fn is_low_confidence(&self) -> bool {
        !self.category.is_certain() && self.confidence < 0.7
    }
}

/// 语体详细度。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Verbosity {
    /// 一句话。
    Concise,
    /// 2~3 句（默认）。
    #[default]
    Standard,
    /// 含变化推演与教学说明。
    Verbose,
}

/// 一条讲解记录。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoachNote {
    /// 第几着（从 1 开始）。
    pub ply: u16,
    /// 走子方。
    pub side: Color,
    /// ICCS 着法串，如 `h2e2`。
    pub mv_iccs: String,
    /// 中文记谱，如「炮二平五」。
    pub notation: String,

    /// 评价等级。
    pub level: MoveLevel,
    /// 分差（厘兵）。
    pub score_loss: i32,
    /// 走子前评分（走子方视角）。
    pub score_before: i32,
    /// 走子后评分（走子方视角）。
    pub score_after: i32,

    /// 命中的战术标签（按置信度降序）。
    pub tactics: Vec<TacticTag>,

    /// 一句话结论。
    pub headline: String,
    /// 详细说明。
    pub detail: String,
    /// 更优着法建议（当等级不是 `Best` 时给出）。
    pub suggestion: Option<String>,

    /// 生成来源。
    pub source: NoteSource,
    /// LLM 增强状态。
    pub llm_status: LlmStatus,

    /// 引擎推荐的主要变例（中文记谱）。
    pub pv: Vec<String>,

    /// 命中的开局名（若处于开局阶段）。
    pub opening_name: Option<String>,
    /// 渲染用的模板 id（便于排查覆盖率与降级）。
    pub template_id: String,
    /// 是否走了兜底模板 —— 兜底**不计入覆盖率**。
    pub used_fallback: bool,
}

impl CoachNote {
    /// 是否为兜底讲解。
    pub fn is_fallback(&self) -> bool {
        self.used_fallback
    }
}
