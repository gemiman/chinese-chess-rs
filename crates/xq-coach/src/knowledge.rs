//! 知识库：编译期内嵌 + 解析。
//!
//! # 为什么用 `include_str!` 而不是运行时读文件
//!
//! [ADR-005](../../../docs/14-决策记录ADR.md#adr-005) 要求领域层零 IO。
//! 素材在**编译期**就进了二进制，运行时不存在任何文件访问 —— 这既满足了约束，
//! 也顺带消灭了「素材路径不对 / 文件缺失」这一整类部署问题。
//!
//! # 配置错误在哪一层被发现
//!
//! | 层次 | 手段 |
//! |---|---|
//! | 编写时 | `scripts/validate_knowledge.py`（CI 中执行，10 类检查） |
//! | 编译时 | `include_str!` 强制素材存在，缺文件直接编译失败 |
//! | 首次使用时 | 本模块解析并 `expect` —— CI 通过则不可能触发 |
//!
//! 这把「配置错误」从**运行时的静默降级**提前到了构建期硬失败。
//! 留到运行时的代价是：表现成「讲解退化成兜底模板」，从日志里几乎看不出原因。

use std::sync::LazyLock;

use serde::Deserialize;

/// 战术模式库（45 条）。
pub const TACTICS_JSON: &str = include_str!("../../../assets/knowledge/tactics.json");
/// 讲解模板库（54 条）。
pub const TEMPLATES_JSON: &str = include_str!("../../../assets/knowledge/coach-templates.json");
/// 开局定式库（10 条）。
pub const OPENINGS_JSON: &str = include_str!("../../../assets/knowledge/openings.json");
/// 棋形约束谓词库（29 个）。
pub const PREDICATES_JSON: &str = include_str!("../../../assets/knowledge/predicates.json");

// ---------------------------------------------------------------- 战术

/// 一条战术定义。
#[derive(Deserialize, Clone, Debug)]
pub struct TacticDef {
    /// 稳定标识，如 `fork`。
    pub id: String,
    /// 中文名。
    pub name: String,
    /// `structure` / `relation` / `formation` / `opening`
    pub category: String,
    /// `L1` / `L2` / `L3`
    pub layer: String,
    /// `builtin`（算法判定）/ `pattern`（棋形匹配）/ `opening_sequence`（开局序列）
    pub detector: String,
    /// 基础置信度。
    #[serde(default = "default_confidence")]
    pub confidence_base: f32,
    /// 说明文案。
    #[serde(default)]
    pub description: String,
    /// 对应模板 id。
    #[serde(default)]
    pub explanation_template: String,
    /// 棋形定义（仅 `pattern` 型有）。
    #[serde(default)]
    pub pattern: Option<PatternDef>,
}

fn default_confidence() -> f32 {
    1.0
}

/// 棋形定义。
#[derive(Deserialize, Clone, Debug)]
pub struct PatternDef {
    /// 攻击者（本步走的那枚棋子）的约束。
    pub attacker: AttackerDef,
    /// 全部必须成立的约束。
    #[serde(default)]
    pub constraints: Vec<ConstraintDef>,
}

/// 攻击者约束。
#[derive(Deserialize, Clone, Debug)]
pub struct AttackerDef {
    /// 棋子种类名，`any` 表示不限。
    pub kind: String,
}

/// 一条棋形约束。
#[derive(Deserialize, Clone, Debug)]
pub struct ConstraintDef {
    /// 谓词 id。
    pub predicate: String,
    /// 部分谓词的 `kind` 参数。
    #[serde(default)]
    pub kind: Option<String>,
    /// 部分谓词的 `min` 参数。
    #[serde(default, rename = "min")]
    pub min: Option<u8>,
    /// 部分谓词的 `max` 参数。
    #[serde(default, rename = "max")]
    pub max: Option<u8>,
    /// 部分谓词的 `distance` 参数。
    #[serde(default)]
    pub distance: Option<u8>,
    /// 部分谓词的 `col` 参数。
    #[serde(default)]
    pub col: Option<u8>,
    /// `attacker_on_rank` 的行偏移参数。
    #[serde(default)]
    pub row_offset_from_enemy_base: Option<u8>,
    /// `attacker_checks_king_within_n_plies` 的步数参数。
    #[serde(default)]
    pub plies: Option<u8>,
}

// ---------------------------------------------------------------- 模板

/// 一个模板。
#[derive(Deserialize, Clone, Debug)]
pub struct TemplateDef {
    /// 模板 id，如 `tpl.check`。
    pub id: String,
    /// 该模板适用的战术 id 列表。为空表示「按等级/兜底」类模板。
    #[serde(default)]
    pub used_by: Vec<String>,
    /// 一句话结论（含占位符）。
    pub headline: String,
    /// 分语体详细说明。
    pub detail: TemplateDetail,
    /// 是否为兜底模板。
    #[serde(default)]
    pub is_fallback: bool,
}

/// 分语体的详细说明。
#[derive(Deserialize, Clone, Debug)]
pub struct TemplateDetail {
    /// 一句话。
    pub concise: String,
    /// 2~3 句。
    pub standard: String,
    /// 含变化推演。
    pub verbose: String,
}

impl TemplateDetail {
    /// 按语体取文本。
    pub fn pick(&self, verbosity: crate::Verbosity) -> &str {
        match verbosity {
            crate::Verbosity::Concise => &self.concise,
            crate::Verbosity::Standard => &self.standard,
            crate::Verbosity::Verbose => &self.verbose,
        }
    }
}

/// 占位符白名单与必需项。
#[derive(Deserialize, Clone, Debug)]
pub struct PlaceholderSpec {
    /// 允许出现的占位符。
    pub allowed: Vec<String>,
    /// 任何模板渲染前都必须有值的占位符。
    pub required_always: Vec<String>,
}

// ---------------------------------------------------------------- 开局

/// 一条开局定式。
#[derive(Deserialize, Clone, Debug)]
pub struct OpeningDef {
    /// 稳定标识。
    pub id: String,
    /// 开局名，如「中炮对屏风马」。
    pub name: String,
    /// 别称。
    #[serde(default)]
    pub aliases: Vec<String>,
    /// ICCS 着法序列。
    pub sequence_iccs: Vec<String>,
    /// 对应的中文记谱序列。
    #[serde(default)]
    pub sequence_names: Vec<String>,
    /// 匹配的最大步数。
    #[serde(default)]
    pub max_ply: u16,
    /// `red` / `black` / `both`。
    #[serde(default)]
    pub side: String,
    /// 一句话结论。
    #[serde(default)]
    pub headline: String,
    /// 红方战略意图。
    #[serde(default)]
    pub idea_red: String,
    /// 黑方战略意图。
    #[serde(default)]
    pub idea_black: String,
    /// 要点提示。
    #[serde(default)]
    pub key_points: Vec<String>,
    /// 标签。
    #[serde(default)]
    pub tags: Vec<String>,
}

// ---------------------------------------------------------------- 顶层

#[derive(Deserialize)]
struct TacticsFile {
    #[serde(default)]
    status: String,
    tactics: Vec<TacticDef>,
}

#[derive(Deserialize)]
struct TemplatesFile {
    #[serde(default)]
    status: String,
    placeholders: PlaceholderSpec,
    templates: Vec<TemplateDef>,
}

#[derive(Deserialize)]
struct OpeningsFile {
    #[serde(default)]
    status: String,
    openings: Vec<OpeningDef>,
}

#[derive(Deserialize)]
struct PredicatesFile {
    #[serde(default)]
    status: String,
    predicates: Vec<PredicateDef>,
}

/// 一条谓词定义（本模块只用它的 id 与参数签名做自检）。
#[derive(Deserialize, Clone, Debug)]
pub struct PredicateDef {
    /// 谓词 id。
    pub id: String,
    /// 参数列表。
    #[serde(default)]
    pub params: Vec<serde_json::Value>,
    /// 含义说明。
    #[serde(default)]
    pub meaning: String,
}

/// 全部知识。
pub struct KnowledgeBase {
    /// 战术库。
    pub tactics: Vec<TacticDef>,
    /// 模板库。
    pub templates: Vec<TemplateDef>,
    /// 开局库。
    pub openings: Vec<OpeningDef>,
    /// 谓词库（用于自检棋形引用的谓词是否存在）。
    pub predicates: Vec<PredicateDef>,
    /// 占位符规范。
    pub placeholders: PlaceholderSpec,
    /// 素材是否仍为「初始」状态（未经棋谱校准）。
    pub is_initial: bool,
}

/// 编译期内嵌的知识库。首次访问时解析。
pub static EMBEDDED: LazyLock<KnowledgeBase> = LazyLock::new(KnowledgeBase::parse_embedded);

impl KnowledgeBase {
    /// 取内嵌知识库。
    pub fn embedded() -> &'static KnowledgeBase {
        &EMBEDDED
    }

    /// 解析内嵌的 JSON。
    ///
    /// 失败即 panic —— 素材在编译期已内嵌，且 CI 的 `validate_knowledge.py` 会
    /// 覆盖全部 schema；能走到这里说明构建产物被污染了，继续跑没有意义。
    fn parse_embedded() -> KnowledgeBase {
        let tactics: TacticsFile = serde_json::from_str(TACTICS_JSON)
            .unwrap_or_else(|e| panic!("内嵌的 tactics.json 无法解析：{e}"));
        let templates: TemplatesFile = serde_json::from_str(TEMPLATES_JSON)
            .unwrap_or_else(|e| panic!("内嵌的 coach-templates.json 无法解析：{e}"));
        let openings: OpeningsFile = serde_json::from_str(OPENINGS_JSON)
            .unwrap_or_else(|e| panic!("内嵌的 openings.json 无法解析：{e}"));
        let predicates: PredicatesFile = serde_json::from_str(PREDICATES_JSON)
            .unwrap_or_else(|e| panic!("内嵌的 predicates.json 无法解析：{e}"));

        let is_initial = tactics.status == "initial"
            || templates.status == "initial"
            || openings.status == "initial"
            || predicates.status == "initial";

        KnowledgeBase {
            tactics: tactics.tactics,
            templates: templates.templates,
            openings: openings.openings,
            predicates: predicates.predicates,
            placeholders: templates.placeholders,
            is_initial,
        }
    }

    /// 按 id 取战术。
    pub fn tactic(&self, id: &str) -> Option<&TacticDef> {
        self.tactics.iter().find(|t| t.id == id)
    }

    /// 按 id 取模板。
    pub fn template(&self, id: &str) -> Option<&TemplateDef> {
        self.templates.iter().find(|t| t.id == id)
    }

    /// 取某战术可用的模板。同一条战术可能对应多个模板（不同语体/场景）。
    pub fn templates_for(&self, tactic_id: &str) -> Vec<&TemplateDef> {
        self.templates
            .iter()
            .filter(|t| t.used_by.iter().any(|u| u == tactic_id))
            .collect()
    }

    /// 兜底模板。
    pub fn fallback_template(&self) -> &TemplateDef {
        self.templates
            .iter()
            .find(|t| t.is_fallback)
            .expect("模板库必须含一条兜底模板（is_fallback=true）")
    }

    /// 统计：战术条数 / 模板条数 / 开局条数 / 谓词条数。
    pub fn sizes(&self) -> (usize, usize, usize, usize) {
        (
            self.tactics.len(),
            self.templates.len(),
            self.openings.len(),
            self.predicates.len(),
        )
    }
}

impl core::fmt::Debug for KnowledgeBase {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (t, tpl, o, p) = self.sizes();
        f.debug_struct("KnowledgeBase")
            .field("tactics", &t)
            .field("templates", &tpl)
            .field("openings", &o)
            .field("predicates", &p)
            .field("is_initial", &self.is_initial)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_knowledge_parses() {
        let kb = KnowledgeBase::embedded();
        assert_eq!(kb.sizes(), (45, 54, 10, 29), "知识库规模与素材统计不符");
    }

    #[test]
    fn fallback_template_exists() {
        let kb = KnowledgeBase::embedded();
        assert!(kb.fallback_template().is_fallback);
    }

    #[test]
    fn every_tactic_template_reference_resolves() {
        let kb = KnowledgeBase::embedded();
        for tactic in &kb.tactics {
            if tactic.explanation_template.is_empty() {
                continue;
            }
            assert!(
                kb.template(&tactic.explanation_template).is_some(),
                "战术 {} 引用了不存在的模板 {}",
                tactic.id,
                tactic.explanation_template
            );
        }
    }

    #[test]
    fn every_pattern_predicate_exists() {
        let kb = KnowledgeBase::embedded();
        for tactic in &kb.tactics {
            let Some(pattern) = &tactic.pattern else {
                continue;
            };
            for constraint in &pattern.constraints {
                assert!(
                    kb.predicates.iter().any(|p| p.id == constraint.predicate),
                    "棋形 {} 引用了未定义的谓词 {}",
                    tactic.id,
                    constraint.predicate
                );
            }
        }
    }

    #[test]
    fn known_initial_state_is_flagged() {
        let kb = KnowledgeBase::embedded();
        assert!(
            kb.is_initial,
            "素材仍是 initial 状态，调用方应据此在 UI 上弱化棋形类标签"
        );
    }
}
