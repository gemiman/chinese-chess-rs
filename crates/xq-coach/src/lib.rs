//! # xq-coach —— 中国象棋战法讲解引擎
//!
//! **这是产品的核心差异化模块。** 竞品的差距不在「能不能下棋」，而在「能不能教人下棋」。
//!
//! ## 双层架构（[ADR-004](../../../docs/14-决策记录ADR.md#adr-004)）
//!
//! ```text
//! 本地路径（零延迟、离线可用、永不失败）
//!   战术识别 → 评价定级 → 模板渲染 → CoachNote
//!
//! LLM 增强（异步、可失败、可降级）
//!   CoachNote → 提示词（只喂结构化事实）→ LLM → 三道校验 → 通过才替换文本
//! ```
//!
//! **LLM 永不在关键路径上**：它失败的唯一后果是少一段润色，用户看到的仍是完整正确的讲解。
//!
//! ## 零 IO 是怎么做到的
//!
//! 知识库（战术 45 / 模板 54 / 开局 10 / 谓词 29）在**编译期**通过 `include_str!`
//! 内嵌，运行时没有任何文件访问。LLM 通过 [`llm::LlmClient`] trait 注入，
//! 实际的 HTTP 实现放在 `xq-server` / `xq-client`。
//!
//! ## 快速上手
//!
//! ```no_run
//! use xq_coach::{Coach, GameContext, Verbosity};
//! use xq_core::Position;
//!
//! let coach = Coach::new();
//! let mut pos = Position::startpos();
//!
//! let mv = pos.from_chinese_notation("炮二平五").unwrap();
//! // root_moves 来自 xq-ai 的搜索结果
//! let root_moves: Vec<(xq_core::Move, i32)> = vec![(mv, 20)];
//!
//! let ctx = GameContext {
//!     history_iccs: &[],
//!     ply: 1,
//!     verbosity: Verbosity::Standard,
//! };
//! let note = coach.analyze(&pos, mv, &root_moves, &ctx);
//!
//! println!("{}", note.headline);
//! println!("{}", note.detail);
//! ```
//!
//! ## 刻意未实现的部分
//!
//! | 能力 | 现状 | 原因 |
//! |---|---|---|
//! | 棋形的多步推演谓词 | `attacker_checks_king_within_n_plies` 只判 1 步 | 多步推演需要再搜一层，与「≤ 5 ms」的本地延迟目标冲突。**宁可少判（少一条棋形标签）也不误判** |
//! | `blockade`（封锁） | 未实现 | 需要比较走子前后对方全部棋子的机动性，代价高而收益低 |
//! | 完整 SEE | 用一层近似（只看「吃它之后对方能否吃回来」） | docs/05 §3.3 已声明首版采用一层近似 |
//!
//! 全部偏差与理由记录在 [docs/05 §12 实现回写](../../../docs/05-战法讲解引擎设计.md)。

#![deny(unsafe_code)]
#![warn(missing_debug_implementations)]

pub mod assess;
pub mod knowledge;
pub mod llm;
pub mod note;
pub mod opening;
pub mod tactic;
pub mod template;

pub use assess::Assessment;
pub use knowledge::KnowledgeBase;
pub use llm::{FactSet, LlmClient, LlmError, LlmRequest, LlmResponse, RejectReason};
pub use note::{CoachNote, LlmStatus, MoveLevel, NoteSource, TacticCategory, TacticTag, Verbosity};
pub use tactic::{MoveContext, TacticDetector};
pub use template::{RenderContext, Rendered};

use xq_core::{Color, GameStatus, Move, Position};

/// 对局上下文。
#[derive(Clone, Debug)]
pub struct GameContext<'a> {
    /// 从第 1 着起的 ICCS 着法串，用于开局匹配。
    pub history_iccs: &'a [String],
    /// 本步是第几着（从 1 开始）。
    pub ply: u16,
    /// 语体详细度。
    pub verbosity: Verbosity,
}

impl Default for GameContext<'_> {
    fn default() -> Self {
        static EMPTY: [String; 0] = [];
        Self {
            history_iccs: &EMPTY,
            ply: 1,
            verbosity: Verbosity::Standard,
        }
    }
}

/// 讲解引擎门面。
pub struct Coach {
    kb: &'static KnowledgeBase,
}

impl Coach {
    /// 新建（知识库在首次调用时解析一次，之后全程复用）。
    pub fn new() -> Self {
        Self {
            kb: KnowledgeBase::embedded(),
        }
    }

    /// 知识库引用。
    pub fn knowledge(&self) -> &'static KnowledgeBase {
        self.kb
    }

    /// 生成讲解（本地路径，同步）。
    ///
    /// # 契约：**永不失败**
    ///
    /// 返回的是 `CoachNote` 而不是 `Result` —— 这是刻意的。讲解是增强功能，
    /// **绝不能成为对局流程的单点故障**。任何内部异常都会降级到兜底讲解。
    pub fn analyze(
        &self,
        pos_before: &Position,
        mv: Move,
        root_moves: &[(Move, i32)],
        ctx: &GameContext<'_>,
    ) -> CoachNote {
        let side = pos_before.side_to_move();
        let enemy = side.opponent();

        // ---- 走子前的信息 ----
        let iccs = pos_before.to_iccs_string(mv);
        let notation = pos_before
            .to_chinese_notation(mv)
            .unwrap_or_else(|_| iccs.clone());
        let captured = pos_before.piece_at(mv.to());

        let mut pre = pos_before.clone();
        let nature = pre.nature_of(mv);
        let was_in_check = pre.is_in_check(side);

        // ---- 走子后的局面 ----
        let pos_after = {
            let mut after = pos_before.clone();
            if after.make_move(mv).is_err() {
                // 非法着法：不 panic，直接给一条兜底讲解
                return self.fallback_note(side, notation, iccs, ctx);
            }
            after
        };

        let played_mates = is_mate_for(&pos_after, enemy);
        let best_mates = root_moves
            .first()
            .map(|(best, _)| {
                let mut probe = pos_before.clone();
                probe.make_move(*best).is_ok() && is_mate_for(&probe, enemy)
            })
            .unwrap_or(false);

        // ---- ① 评价定级 ----
        let assessment = assess::assess(root_moves, mv, played_mates, best_mates);

        // ---- ② 战术识别 ----
        let move_ctx = MoveContext {
            side,
            mv,
            nature,
            was_in_check,
            captured,
            score_best: assessment.score_before,
            score_loss: assessment.score_loss,
        };
        let detector = TacticDetector::new(self.kb);
        let mut tactics = detector.detect(pos_before, &pos_after, &move_ctx);

        // ---- 开局匹配 ----
        let opening = opening::match_opening(self.kb, ctx.history_iccs);
        if let Some(matched) = &opening {
            // 尽量复用知识库里已有的开局战术 id —— 这样能命中它的专用模板，
            // 而不是落到「按等级」的通用模板。匹配方式是**开局名包含关系**：
            // 「中炮对屏风马」包含「中炮」，于是复用 `opening_central_cannon`。
            let (id, name) = self.resolve_opening_tactic(&matched.opening.name);
            if !tactics.iter().any(|t| t.id == id) {
                tactics.push(TacticTag {
                    id,
                    name,
                    category: TacticCategory::Opening,
                    confidence: if matched.fully_played { 0.9 } else { 0.75 },
                });
            }
        }

        // ---- ③ 模板渲染 ----
        let best_notation = assessment
            .best_move
            .and_then(|best| pos_before.to_chinese_notation(best).ok());
        let suggestion = if assessment.level == MoveLevel::Best {
            None
        } else {
            best_notation.clone().map(|n| {
                format!(
                    "引擎推荐 {n}（评分比本步高 {} 厘兵）",
                    assessment.score_loss
                )
            })
        };

        let rendered = template::render(
            self.kb,
            &self.build_render_context(
                side,
                &notation,
                captured,
                &assessment,
                best_notation.as_deref(),
                opening.as_ref().map(|m| m.opening.name.as_str()),
                &tactics,
            ),
            assessment.level,
            &tactics,
            ctx.verbosity,
        );

        CoachNote {
            ply: ctx.ply,
            side,
            mv_iccs: iccs,
            notation,
            level: assessment.level,
            score_loss: assessment.score_loss,
            score_before: assessment.score_before,
            score_after: assessment.score_after,
            tactics,
            headline: rendered.headline,
            detail: rendered.detail,
            suggestion,
            source: NoteSource::Local,
            llm_status: LlmStatus::None,
            pv: Vec::new(),
            opening_name: opening.map(|m| m.opening.name.clone()),
            template_id: rendered.template_id,
            used_fallback: rendered.used_fallback,
        }
    }

    /// 把开局定式名映射到知识库里的开局战术 id。
    ///
    /// 匹配规则：取**名字最长**的那个被包含的开局战术。这样「中炮对屏风马」会命中
    /// 「中炮」而不是别的更短的巧合匹配；一条都匹配不上时退回用定式名本身拼一个 id
    /// （此时标签仍会展示，只是会走「按等级」的通用模板）。
    fn resolve_opening_tactic(&self, opening_name: &str) -> (String, String) {
        let mut best: Option<(&str, &str)> = None;
        for tactic in &self.kb.tactics {
            if tactic.detector != "opening_sequence" {
                continue;
            }
            if !opening_name.contains(&tactic.name) {
                continue;
            }
            if best.is_none_or(|(_, name)| tactic.name.len() > name.len()) {
                best = Some((tactic.id.as_str(), tactic.name.as_str()));
            }
        }
        match best {
            Some((id, name)) => (id.to_string(), name.to_string()),
            None => (
                format!("opening_seq_{opening_name}"),
                opening_name.to_string(),
            ),
        }
    }

    /// 组装渲染上下文。**全部 12 个占位符都必须有值**（缺失的用语义合理的默认文本）。
    #[allow(clippy::too_many_arguments)]
    fn build_render_context(
        &self,
        side: Color,
        notation: &str,
        captured: u8,
        assessment: &assess::Assessment,
        best_notation: Option<&str>,
        opening_name: Option<&str>,
        tactics: &[TacticTag],
    ) -> RenderContext {
        let piece_word = |piece: u8| -> String {
            xq_core::kind_of(piece)
                .map(|k| {
                    let color = xq_core::color_of(piece).unwrap_or(Color::Red);
                    k.name_zh(color).to_string()
                })
                .unwrap_or_default()
        };

        let tactic_names = tactics
            .iter()
            .take(3)
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join("、");

        RenderContext {
            side: side.name_zh().to_string(),
            notation: notation.to_string(),
            piece_name: "棋子".to_string(), // 由调用方在需要时覆盖
            captured_piece: if captured == xq_core::EMPTY {
                "棋子".to_string()
            } else {
                piece_word(captured)
            },
            best_move_notation: best_notation.unwrap_or(notation).to_string(),
            score_loss: assessment.score_loss.to_string(),
            level_text: assessment.level.label().to_string(),
            opening_name: opening_name.unwrap_or("当前布局").to_string(),
            tactic_names: if tactic_names.is_empty() {
                assessment.level.label().to_string()
            } else {
                tactic_names
            },
            best_reply_hint: best_notation
                .map(|n| format!("走 {n}"))
                .unwrap_or_else(|| "另寻它法".to_string()),
            threat_targets: "对方棋子".to_string(),
            defended_piece: "己方棋子".to_string(),
        }
    }

    /// 兜底讲解：任何异常路径都落到这里，保证 `analyze` 永不失败。
    fn fallback_note(
        &self,
        side: Color,
        notation: String,
        iccs: String,
        ctx: &GameContext<'_>,
    ) -> CoachNote {
        CoachNote {
            ply: ctx.ply,
            side,
            mv_iccs: iccs,
            notation: notation.clone(),
            level: MoveLevel::Good,
            score_loss: 0,
            score_before: 0,
            score_after: 0,
            tactics: Vec::new(),
            headline: format!("{}方 {notation}。", side.name_zh()),
            detail: String::new(),
            suggestion: None,
            source: NoteSource::Local,
            llm_status: LlmStatus::None,
            pv: Vec::new(),
            opening_name: None,
            template_id: "builtin.last_resort".to_string(),
            used_fallback: true,
        }
    }

    /// 请求 LLM 增强。
    ///
    /// 失败 / 超时 / 校验不过时，**原样返回本地讲解**，只把 `llm_status` 标为
    /// [`LlmStatus::Failed`] —— 用户完全无感。
    pub fn enhance(&self, note: &CoachNote, client: &dyn LlmClient) -> CoachNote {
        // 只对需要解读的等级调用 —— 这同时降低成本与提升价值
        if !note.level.needs_explanation() {
            return note.clone();
        }

        let facts = FactSet::from_note(note);
        let request = LlmRequest {
            system_prompt: llm::SYSTEM_PROMPT.to_string(),
            user_prompt: llm::build_user_prompt(note, &[]),
            max_tokens: 256,
            temperature: 0.3,
            timeout_ms: 3_000,
        };

        let mut enhanced = note.clone();
        match client.complete(&request) {
            Ok(response) => match llm::validate_output(&response.text, &facts) {
                Ok(()) => {
                    enhanced.detail = response.text.trim().to_string();
                    enhanced.source = NoteSource::Llm;
                    enhanced.llm_status = LlmStatus::Enhanced;
                }
                Err(_reason) => {
                    // 校验不过 → 丢弃 LLM 输出，保留本地讲解
                    enhanced.llm_status = LlmStatus::Failed;
                }
            },
            Err(_) => {
                enhanced.llm_status = LlmStatus::Failed;
            }
        }
        enhanced
    }
}

impl Default for Coach {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for Coach {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Coach").field("knowledge", self.kb).finish()
    }
}

/// 走完 `mv` 之后，`loser` 方是否被将死。
fn is_mate_for(pos_after: &Position, loser: Color) -> bool {
    let mut probe = pos_after.clone();
    matches!(
        probe.status(),
        GameStatus::Checkmate { loser: l } if l == loser
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use xq_core::square::from_iccs;

    fn mv(iccs: &str) -> Move {
        Move::new(
            from_iccs(&iccs[0..2]).unwrap(),
            from_iccs(&iccs[2..4]).unwrap(),
        )
    }

    /// 初始局面的全部 44 步都必须能生成非空讲解，且不得残留占位符。
    #[test]
    fn every_startpos_move_produces_a_readable_note() {
        let coach = Coach::new();
        let mut pos = Position::startpos();

        let mut best = 0;
        for mv in pos.legal_moves() {
            let roots: Vec<(Move, i32)> = vec![(mv, best)];
            best -= 5; // 让每一步都成为「最优」，避免全部落到 Missed
            let note = coach.analyze(&pos, mv, &roots, &GameContext::default());

            assert!(!note.headline.is_empty(), "着法 {mv} 的 headline 为空");
            assert!(
                !note.headline.contains('{') && !note.headline.contains('}'),
                "着法 {mv} 的 headline 残留占位符：{}",
                note.headline
            );
            assert!(
                !note.detail.contains('{') && !note.detail.contains('}'),
                "着法 {mv} 的 detail 残留占位符：{}",
                note.detail
            );
            assert_eq!(note.side, Color::Red);
        }
    }

    #[test]
    fn opening_move_is_recognized() {
        let coach = Coach::new();
        let pos = Position::startpos();
        let m = mv("h2e2");
        let roots = vec![(m, 20)];
        let note = coach.analyze(&pos, m, &roots, &GameContext::default());

        assert_eq!(note.notation, "炮二平五");
        assert_eq!(note.level, MoveLevel::Best);
        assert!(!note.used_fallback, "开局着法不应落到兜底模板");
        // 应当识别出「中炮」这类开局战术
        assert!(
            note.tactics
                .iter()
                .any(|t| t.category == TacticCategory::Opening),
            "应识别出开局类战术，实际 {:?}",
            note.tactics
        );
    }

    /// 分差大时应给出更优着法建议。
    #[test]
    fn bad_move_gets_a_suggestion() {
        let coach = Coach::new();
        let pos = Position::startpos();
        let good = mv("h2e2");
        let bad = mv("a0a1");
        let roots = vec![(good, 200), (bad, -300)];
        let note = coach.analyze(&pos, bad, &roots, &GameContext::default());

        assert!(note.level != MoveLevel::Best);
        assert!(note.suggestion.is_some(), "非最优着法应给出建议");
        assert!(note.suggestion.unwrap().contains("炮二平五"));
    }

    /// 非法着法不得 panic，而是给出兜底讲解。
    #[test]
    fn illegal_move_does_not_panic() {
        let coach = Coach::new();
        let pos = Position::startpos();
        // a0 → a5 被 a3 的红兵挡住，不合法
        let bad = mv("a0a5");
        let note = coach.analyze(&pos, bad, &[], &GameContext::default());
        assert!(note.used_fallback);
        assert!(!note.headline.is_empty());
    }

    /// 空 root_moves 不得 panic。
    #[test]
    fn empty_root_moves_does_not_panic() {
        let coach = Coach::new();
        let pos = Position::startpos();
        let note = coach.analyze(&pos, mv("h2e2"), &[], &GameContext::default());
        assert!(!note.headline.is_empty());
    }

    /// **不失败保证**：大量随机着法调用 `analyze`，断言永不 panic、永不残留占位符。
    #[test]
    fn analyze_never_fails_on_random_play() {
        let coach = Coach::new();
        let mut pos = Position::startpos();
        let mut history: Vec<String> = Vec::new();
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        let mut ply = 0u16;

        for _ in 0..300 {
            if pos.status().is_over() {
                break;
            }
            let moves = pos.legal_moves();
            if moves.is_empty() {
                break;
            }
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let pick = moves[(state % moves.len() as u64) as usize];

            // 构造一组「分差递增」的根着法评分
            let roots: Vec<(Move, i32)> = moves
                .iter()
                .enumerate()
                .map(|(i, m)| (*m, 100 - (i as i32) * 7))
                .collect();

            ply += 1;
            let ctx = GameContext {
                history_iccs: &history,
                ply,
                verbosity: Verbosity::Standard,
            };
            let note = coach.analyze(&pos, pick, &roots, &ctx);

            assert!(!note.headline.is_empty());
            assert!(
                !note.headline.contains('{'),
                "残留占位符：{}",
                note.headline
            );
            assert!(!note.detail.contains('{'));

            history.push(pos.to_iccs_string(pick));
            pos.make_move(pick).expect("合法着法应能被接受");
        }
        assert!(ply > 10, "随机对局应能走若干步，实际 {ply}");
    }
}
