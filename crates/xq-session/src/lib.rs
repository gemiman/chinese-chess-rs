//! 对局会话层与传输对象。
//!
//! # 它是什么
//!
//! 一局对局的完整状态机（局面、记谱、上一着），加上两个门面：
//! [`AppState::engine_move`] / [`AppState::engine_hint`]（引擎）与
//! [`AppState::coach_last_move`]（讲解）。以及**对前端的 JSON 契约**。
//!
//! # 为什么不放在 xq-bridge 里
//!
//! 因为宿主有两个：开发期的 HTTP 桥（`xq-bridge`）与桌面客户端（`xq-client`）。
//! 如果这段逻辑留在桥里，Tauri 端就得复制一份 DTO —— 两份契约各自演进，
//! 迟早对不上。抽出来之后，**两端都只包一层薄薄的适配**。
//!
//! # 为什么 DTO 与内核类型分开
//!
//! 前端用 `col/row` 定位棋盘，内核用一维索引；两边各自保持最顺手的形状，
//! 转换只发生在这一个文件里。这样换传输格式（Tauri command、WebSocket）
//! 时，改动不会渗透到 `xq-core`。
//!
//! # 一个容易写错的地方
//!
//! **中文记谱必须在走子之前生成。** `describe()` 要靠当前局面判断「前 / 后」
//! 消歧，走完子之后再算就会拿到错误的盘面。因此 [`Game::apply`] 先算记谱、
//! 再落子，并把算好的 [`PlayedMove`] 存下来 —— 而不是事后重算。

pub mod clock;

pub use clock::{Clock, ClockSnapshot, TimeControl, TimeControlInput, Timeout};

use serde::Serialize;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use xq_ai::{AtomicStop, Difficulty, Engine, StopSignal};
use xq_coach::{Coach, GameContext, Verbosity};
use xq_core::piece::{color_of, kind_of};
use xq_core::square::{BOARD_SIZE, col_of, row_of, to_iccs};
use xq_core::{Color, GameStatus, Move, MoveNature, PieceKind, Position};

/// 全局应用状态。本地单用户开发工具，只维护一局。
pub struct AppState {
    game: Mutex<Game>,
    /// 引擎实例独立加锁。
    ///
    /// **不要和 `game` 共用一把锁** —— 搜索可能耗时数秒，共用会导致这期间的
    /// 任何状态查询（前端每次轮询）都被阻塞，界面卡死。
    engine: Mutex<EngineSlot>,
    /// 讲解引擎。无状态（知识库编译期内嵌、只读），因此**不需要加锁**。
    coach: Coach,
}

struct EngineSlot {
    engine: Engine,
    /// 上次使用的档位，用于在响应里回显。
    level: Difficulty,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            game: Mutex::new(Game::new()),
            engine: Mutex::new(EngineSlot {
                engine: Engine::new(0x5EED_1234),
                level: Difficulty::L3,
            }),
            coach: Coach::new(),
        }
    }

    /// 讲解引擎引用。
    #[allow(dead_code)]
    pub fn coach(&self) -> &Coach {
        &self.coach
    }

    /// 在锁内执行一段操作。
    ///
    /// ⚠️ 闭包内**不要**再调用 `with_game` —— `Mutex` 不可重入，会死锁。
    pub fn with_game<R>(&self, f: impl FnOnce(&mut Game) -> R) -> R {
        let mut guard = self.game.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    /// 换局时同时重置引擎（清空置换表，避免上一局的数据污染）。
    pub fn reset_engine(&self) {
        let mut slot = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        slot.engine.new_game();
    }

    /// 让引擎走一步。
    pub fn engine_move(
        &self,
        level: Difficulty,
        think_ms: u64,
    ) -> Result<EngineMoveOutcome, String> {
        // 有棋钟时，思考时间要被本方剩余时间封顶 ——
        // 否则引擎会「想太久」把自己走成超时判负，那显然不合理。
        // 留 250ms 余量给落子本身，别卡在最后一毫秒上。
        let think_ms = match self.with_game(|game| game.step_left_ms()) {
            Some(left) => think_ms.min(left.saturating_sub(250).clamp(50, i64::MAX) as u64),
            None => think_ms,
        };

        // ① 取局面快照（短锁）
        let mut pos = self.with_game(|game| game.snapshot());

        // ② 在锁外搜索（这是长耗时步骤，不能占着 game 锁）
        let started = Instant::now();
        let (result, _) = self.run_search(&mut pos, level, think_ms);
        let elapsed = started.elapsed().as_millis() as u64;

        // ③ 回到锁内落子（短锁）
        let Some(mv) = result.best_move else {
            return Err("当前局面无着可走".to_string());
        };

        let info = EngineInfo {
            level: level.id().to_string(),
            level_label: level.label().to_string(),
            depth: result.depth,
            score: result.score,
            nodes: result.nodes,
            mate_in: result.mate_distance(),
            stopped: result.stopped,
            think_ms: elapsed,
        };
        let played = self.with_game(|game| game.apply_known_move(mv))?;
        Ok(EngineMoveOutcome { played, info })
    }

    /// 只取推荐着法，不落子。
    pub fn engine_hint(
        &self,
        level: Difficulty,
        think_ms: u64,
        count: usize,
    ) -> Result<HintOutcome, String> {
        let mut pos = self.with_game(|game| game.snapshot());
        let started = Instant::now();
        let (result, pos_after) = self.run_search(&mut pos, level, think_ms);
        let elapsed = started.elapsed().as_millis() as u64;

        let suggestions: Vec<HintItem> = result
            .root_moves
            .iter()
            .take(count.clamp(1, 10))
            .map(|(mv, score)| {
                let notation = pos_after
                    .to_chinese_notation(*mv)
                    .unwrap_or_else(|_| pos_after.to_iccs_string(*mv));
                HintItem {
                    from: to_iccs(mv.from()),
                    to: to_iccs(mv.to()),
                    iccs: pos_after.to_iccs_string(*mv),
                    notation,
                    score: *score,
                    capture: pos_after.piece_at(mv.to()) != xq_core::EMPTY,
                }
            })
            .collect();

        Ok(HintOutcome {
            suggestions,
            info: EngineInfo {
                level: level.id().to_string(),
                level_label: level.label().to_string(),
                depth: result.depth,
                score: result.score,
                nodes: result.nodes,
                mate_in: result.mate_distance(),
                stopped: result.stopped,
                think_ms: elapsed,
            },
        })
    }

    /// 执行一次搜索。返回结果与搜索后的局面（局面会被还原，可继续使用）。
    fn run_search(
        &self,
        pos: &mut Position,
        level: Difficulty,
        think_ms: u64,
    ) -> (xq_ai::SearchResult, Position) {
        let mut slot = self.engine.lock().unwrap_or_else(|e| e.into_inner());
        slot.engine.set_difficulty(level);
        slot.level = level;

        // 用一个看门线程在 think_ms 后请求停止。
        //
        // 这就是「零 IO 约束」的落地点：`xq-ai` 自己不读时钟，时间控制由调用方
        // 以 StopSignal 的形式注入。正式产品里这个实现会放在客户端/服务端。
        let stop = Arc::new(AtomicStop::new());
        let watchdog = {
            let stop = Arc::clone(&stop);
            let ms = think_ms.max(1);
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(ms));
                stop.request_stop();
            })
        };

        // 记下搜索前的层数：搜索的契约是「用完把局面还原成进来的样子」，
        // 而不是「还原成开局」。两者只在开局才恰好相等。
        let ply_before = pos.ply();
        let result = slot.engine.search(pos, &*stop as &dyn StopSignal);
        // 看门线程会自然结束（睡醒后置一次标志即退出），无需 join
        drop(watchdog);

        // 搜索结束后局面必须已还原 —— 这是搜索的基本契约。
        //
        // ⚠️ 这里**不能**断言 `ply() == 0`。讲解（`coach_last_move`）传进来的局面是
        // 「上一着走之前」的克隆，层数本来就不为 0；引擎应招传进来的也是当前局面的
        // 快照。原先写死 0，效果是**正常时误报、真出问题时又不查**：debug 版只要走
        // 过一步再让引擎应招就 panic（前端表现为「响应中途断掉」），而 release 版
        // 因为 `debug_assert!` 被编译掉，还原契约实际上从未被验证过。
        debug_assert_eq!(pos.ply(), ply_before, "搜索结束后局面应被完全还原");

        (result, pos.clone())
    }

    /// 生成最后一步的战法讲解。
    ///
    /// 需要一次搜索来拿 `root_moves`（评价定级的唯一依据），因此这一步比纯本地
    /// 模板渲染慢 —— 所以它走**独立接口**、由前端在走子后异步请求，
    /// 绝不阻塞走棋路径（docs/05 §6.5 的异步时序）。
    pub fn coach_last_move(
        &self,
        level: Difficulty,
        think_ms: u64,
    ) -> Result<(xq_coach::CoachNote, EngineInfo), String> {
        let (mut pos_before, mv, history) = self
            .with_game(|game| game.last_move_context())
            .ok_or_else(|| "还没有走过任何着法".to_string())?;

        let ply = history.len() as u16;
        let started = Instant::now();
        let (result, _) = self.run_search(&mut pos_before, level, think_ms);
        let elapsed = started.elapsed().as_millis() as u64;

        let ctx = GameContext {
            history_iccs: &history,
            ply,
            verbosity: Verbosity::Standard,
        };
        let note = self
            .coach
            .analyze(&pos_before, mv, &result.root_moves, &ctx);

        let info = EngineInfo {
            level: level.id().to_string(),
            level_label: level.label().to_string(),
            depth: result.depth,
            score: result.score,
            nodes: result.nodes,
            mate_in: result.mate_distance(),
            stopped: result.stopped,
            think_ms: elapsed,
        };
        Ok((note, info))
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// 一局对局。
pub struct Game {
    pos: Position,
    /// 已走着的完整记录（含走子前算好的记谱）。
    log: Vec<PlayedMove>,
    /// 上一着。
    last: Option<PlayedMove>,
    /// 棋钟。`None` 表示不限时。
    clock: Option<Clock>,
    /// 因超时产生的终局。
    ///
    /// 超时**不是**规则产生的终局，所以不能塞进 `xq-core::GameStatus`
    /// —— 内核只认棋盘上的事实，不认「谁的表走完了」。
    timeout: Option<Timeout>,
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    pub fn new() -> Self {
        Self {
            pos: Position::startpos(),
            log: Vec::new(),
            last: None,
            clock: None,
            timeout: None,
        }
    }

    /// 按限时配置开一局。配置为不限时时等价于 [`Game::new`]。
    pub fn new_with_clock(cfg: TimeControl) -> Self {
        let mut game = Self::new();
        game.set_time_control(cfg);
        game
    }

    /// 设定限时并重新计时。棋钟**在开局时就走** ——
    /// 与真实棋赛一致：摆好钟按下开始，红方就开始用时了。
    pub fn set_time_control(&mut self, cfg: TimeControl) {
        self.clock = if cfg.is_unlimited() {
            None
        } else {
            let mut clock = Clock::new(cfg);
            clock.start(Instant::now());
            Some(clock)
        };
        self.timeout = None;
    }

    /// 当前限时配置。
    pub fn time_control(&self) -> TimeControl {
        self.clock
            .as_ref()
            .map(|c| c.config())
            .unwrap_or(TimeControl::UNLIMITED)
    }

    /// 归属方本步还剩多少毫秒。不限时时返回 `None`。
    pub fn step_left_ms(&self) -> Option<i64> {
        let clock = self.clock.as_ref()?;
        Some(clock.step_left_ms(self.pos.side_to_move(), Instant::now()))
    }

    /// 是否已因超时终局。
    pub fn timeout_loser(&self) -> Option<Color> {
        self.timeout.map(|t| t.loser)
    }

    /// 重开一局，保留限时配置。
    pub fn reset(&mut self) {
        *self = Game::new_with_clock(self.time_control());
    }

    /// 局面快照。供引擎在**锁外**搜索 —— 搜索可能耗时数秒，不能占着锁。
    pub fn snapshot(&self) -> Position {
        self.pos.clone()
    }

    /// 落下一个已经过校验的着法。
    pub fn apply_known_move(&mut self, mv: Move) -> Result<PlayedMove, String> {
        if !self.pos.is_legal(mv) {
            return Err(format!("着法 {mv} 在当前局面下不合法"));
        }
        self.apply_move(mv)
    }

    /// 最后一步的「讲解上下文」：走子**前**的局面 + 那一步 + 到该步为止的完整 ICCS 序列。
    ///
    /// 讲解必须拿**走子前**的局面算 —— 记谱、战术识别、吃子判定全都依赖它。
    /// 做法是把最后一步从当前局面上撤掉，而不是另存快照：这样只有一个真相源，
    /// 不会出现「快照与棋盘不同步」的隐蔽 bug。
    pub fn last_move_context(&self) -> Option<(Position, Move, Vec<String>)> {
        let record = *self.pos.move_stack().last()?;
        let mut before = self.pos.clone();
        before.unmake_move()?;

        // 开局匹配看的是**含本步在内**的完整序列 —— 定式刚好走完时才能被识别出来
        let history: Vec<String> = self.log.iter().map(|entry| entry.iccs.clone()).collect();
        Some((before, record.mv, history))
    }

    pub fn load_fen(&mut self, fen: &str) -> Result<(), String> {
        let pos = Position::from_fen(fen).map_err(|e| e.to_string())?;
        self.pos = pos;
        self.log.clear();
        self.last = None;
        self.timeout = None;
        // 换局面等于换一局，棋钟回到满血重走
        let cfg = self.time_control();
        if !cfg.is_unlimited() {
            self.set_time_control(cfg);
        }
        Ok(())
    }

    /// 走一步。`from` / `to` 是一维索引。
    pub fn apply(&mut self, from: u8, to: u8) -> Result<PlayedMove, String> {
        let Some(mv) = self.pos.legal_move_to(from, to) else {
            return Err(format!(
                "{} → {} 不是当前局面下的合法着法",
                to_iccs(from),
                to_iccs(to)
            ));
        };
        self.apply_move(mv)
    }

    /// 按 ICCS 坐标走一步，如 `apply_iccs("h2", "e2")`。
    ///
    /// **坐标解析必须留在会话层。** 两个宿主（HTTP 桥、Tauri command）拿到的都是
    /// 字符串，如果各自解析一遍，就会出现「同一串坐标在浏览器里合法、在桌面端报错」
    /// 这种极难定位的差异；更糟的是适配层会被迫依赖 `xq-core`，把领域类型泄漏到
    /// 传输层。`xq-bridge` 曾因此漏声明依赖而**整个 workspace 编译不过**。
    pub fn apply_iccs(&mut self, from: &str, to: &str) -> Result<PlayedMove, String> {
        let from_idx = xq_core::from_iccs(from).ok_or_else(|| format!("非法坐标：{from}"))?;
        let to_idx = xq_core::from_iccs(to).ok_or_else(|| format!("非法坐标：{to}"))?;
        self.apply(from_idx, to_idx)
    }

    /// 按中文记谱或 ICCS 走一步。
    pub fn apply_text(&mut self, text: &str) -> Result<PlayedMove, String> {
        let mv = self.resolve_text(text)?;
        self.apply_move(mv)
    }

    /// 把文本解析成着法，但**不落子**（便于先解析后计算记谱）。
    fn resolve_text(&self, text: &str) -> Result<Move, String> {
        let trimmed = text.trim();
        let compact: String = trimmed
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-' && *c != '>')
            .collect();
        if compact.len() == 4
            && compact.is_ascii()
            && let (Some(from), Some(to)) = (
                xq_core::from_iccs(&compact[0..2]),
                xq_core::from_iccs(&compact[2..4]),
            )
        {
            return Ok(Move::new(from, to));
        }
        // from_chinese_notation 需要 &mut（内部做合法着法生成），这里用临时克隆
        let mut probe = self.pos.clone();
        probe
            .from_chinese_notation(trimmed)
            .map_err(|e| e.to_string())
    }

    fn apply_move(&mut self, mv: Move) -> Result<PlayedMove, String> {
        // ⓪ 先结算棋钟。超时的话这一步不算数 —— 钟停、判负，局面保持不动。
        if let Some(t) = self.timeout {
            return Err(format!("对局已因超时结束（{}方负）", t.loser.name_zh()));
        }
        let side = self.pos.side_to_move();
        if let Some(clock) = self.clock.as_mut() {
            let now = Instant::now();
            if clock.settle_move(side, now).is_err() {
                clock.stop();
                self.timeout = Some(Timeout { loser: side });
                return Err(format!("{}方超时判负", side.name_zh()));
            }
            // 走完这一步就轮到对方，从此刻开始为对方走秒
            clock.start(now);
        }

        // ① 记谱必须在落子前算（「前 / 后」消歧依赖当前盘面）
        let notation = self
            .pos
            .to_chinese_notation(mv)
            .unwrap_or_else(|_| format!("{}{}", to_iccs(mv.from()), to_iccs(mv.to())));
        let iccs = self.pos.to_iccs_string(mv);
        let capture = self.pos.piece_at(mv.to()) != xq_core::EMPTY;
        let side = color_word(self.pos.side_to_move()).to_string();

        // ② 算性质（nature_of 内部自己做 make/unmake，不影响局面）
        let nature = self.pos.nature_of(mv);

        // ③ 落子（顺带做权威裁决）
        self.pos.make_move(mv).map_err(|e| e.to_string())?;

        let played = PlayedMove {
            from: to_iccs(mv.from()),
            to: to_iccs(mv.to()),
            iccs,
            notation,
            capture,
            nature: nature_word(nature).to_string(),
            nature_text: nature.name_zh().to_string(),
            side,
        };
        self.last = Some(played.clone());
        self.log.push(played.clone());
        Ok(played)
    }

    /// 悔一步。棋钟一并回退 —— 否则悔棋就成了「白送回时间」。
    pub fn undo(&mut self) -> bool {
        if self.pos.unmake_move().is_none() {
            return false;
        }
        self.log.pop();
        self.last = self.log.last().cloned();
        // 悔棋同样撤销超时终局
        self.timeout = None;
        if let Some(clock) = self.clock.as_mut() {
            clock.undo();
            clock.start(Instant::now());
        }
        true
    }

    /// 组装前端需要的完整状态。
    pub fn build_dto(&mut self) -> StateDto {
        // 超时优先：它是「非规则终局」，棋盘上未必看得出，只能由会话层给出
        let status_dto = match self.timeout {
            Some(t) => StatusDto::timeout(t.loser),
            None => StatusDto::from(self.pos.status(), &mut self.pos),
        };

        // 棋钟快照。顺带 start()：玩家第一次看到这个局面，就是他开始用时的时刻。
        // 重复调用是空操作，不会把起点冲掉。
        let to_move = self.pos.side_to_move();
        let clock_dto = match self.clock.as_mut() {
            Some(clock) => {
                let now = Instant::now();
                clock.start(now);
                Some(clock.snapshot(to_move, now))
            }
            None => None,
        };

        let pieces: Vec<PieceDto> = (0..BOARD_SIZE as u8)
            .filter_map(|idx| {
                let piece = self.pos.piece_at(idx);
                if piece == xq_core::EMPTY {
                    return None;
                }
                let color = color_of(piece).expect("非空棋子必有颜色");
                let kind = kind_of(piece).expect("非空棋子必有种类");
                Some(PieceDto {
                    sq: to_iccs(idx),
                    col: col_of(idx),
                    row: row_of(idx),
                    color: color_word(color).to_string(),
                    kind: kind_word(kind).to_string(),
                    sprite: sprite_name(color, kind),
                    glyph: kind.name_zh(color).to_string(),
                })
            })
            .collect();

        // 合法着法列表：基于当前局面生成，describe() 用的正是当前盘面，故无需 make/unmake
        let legal: Vec<MoveOption> = self
            .pos
            .legal_moves()
            .into_iter()
            .map(|mv| {
                let notation = self
                    .pos
                    .to_chinese_notation(mv)
                    .unwrap_or_else(|_| self.pos.to_iccs_string(mv));
                MoveOption {
                    from: to_iccs(mv.from()),
                    to: to_iccs(mv.to()),
                    iccs: self.pos.to_iccs_string(mv),
                    notation,
                    capture: self.pos.piece_at(mv.to()) != xq_core::EMPTY,
                }
            })
            .collect();

        StateDto {
            fen: self.pos.to_fen(),
            side: color_word(self.pos.side_to_move()).to_string(),
            in_check: self.pos.is_in_check(self.pos.side_to_move()),
            status: status_dto,
            pieces,
            legal,
            last_move: self.last.clone(),
            history: self.log.clone(),
            halfmove_clock: self.pos.halfmove_clock(),
            fullmove_number: self.pos.fullmove_number(),
            clock: clock_dto,
        }
    }
}

fn color_word(color: Color) -> &'static str {
    match color {
        Color::Red => "red",
        Color::Black => "black",
    }
}

fn kind_word(kind: PieceKind) -> &'static str {
    match kind {
        PieceKind::King => "king",
        PieceKind::Advisor => "advisor",
        PieceKind::Elephant => "elephant",
        PieceKind::Horse => "horse",
        PieceKind::Chariot => "chariot",
        PieceKind::Cannon => "cannon",
        PieceKind::Pawn => "pawn",
    }
}

/// 对应 `assets/pieces/<sprite>.svg` 的文件名主干。
///
/// 黑方的「将」用的是 `general` 而非 `king`，与素材文件名保持一致；
/// 这个映射只在这里维护一处。
fn sprite_name(color: Color, kind: PieceKind) -> String {
    let prefix = match color {
        Color::Red => "red",
        Color::Black => "black",
    };
    let suffix = match (color, kind) {
        (Color::Red, PieceKind::King) => "king",
        (Color::Black, PieceKind::King) => "general",
        (_, PieceKind::Advisor) => "advisor",
        (_, PieceKind::Elephant) => "elephant",
        (_, PieceKind::Horse) => "horse",
        (_, PieceKind::Chariot) => "chariot",
        (_, PieceKind::Cannon) => "cannon",
        (_, PieceKind::Pawn) => "pawn",
    };
    format!("{prefix}_{suffix}")
}

fn nature_word(nature: MoveNature) -> &'static str {
    match nature {
        MoveNature::Check => "check",
        MoveNature::Capture => "capture",
        MoveNature::Chase => "chase",
        MoveNature::Exchange => "exchange",
        MoveNature::Interpose => "interpose",
        MoveNature::Escape => "escape",
        MoveNature::CaptureAttacker => "capture_attacker",
        MoveNature::Idle => "idle",
    }
}

// ------------------------------------------------------------------ DTO

#[derive(Serialize)]
pub struct StateDto {
    pub fen: String,
    pub side: String,
    pub in_check: bool,
    pub status: StatusDto,
    pub pieces: Vec<PieceDto>,
    pub legal: Vec<MoveOption>,
    pub last_move: Option<PlayedMove>,
    pub history: Vec<PlayedMove>,
    pub halfmove_clock: u16,
    pub fullmove_number: u16,
    /// 棋钟。`null` 表示这一局不限时。
    pub clock: Option<ClockSnapshot>,
}

#[derive(Serialize)]
pub struct StatusDto {
    /// `ongoing` | `check` | `checkmate` | `stalemate` | `draw`
    pub kind: String,
    pub text: String,
    pub over: bool,
    /// 将死 / 困毙时的负方；和棋或进行中为 `null`。
    pub loser: Option<String>,
}

impl StatusDto {
    /// 超时判负。
    ///
    /// 超时**不是棋盘上的事实** —— 局面看起来可能完全正常，所以只能由会话层
    /// 单独构造，`xq-core::GameStatus` 里没有也不该有这个变体。
    fn timeout(loser: Color) -> Self {
        Self {
            kind: "timeout".to_string(),
            text: format!("{}方超时，判负", loser.name_zh()),
            over: true,
            loser: Some(color_word(loser).to_string()),
        }
    }

    fn from(status: GameStatus, pos: &mut Position) -> Self {
        let (kind, text, loser) = match status {
            GameStatus::Ongoing => ("ongoing", "进行中".to_string(), None),
            GameStatus::Check { side } => (
                "check",
                format!("{}方被将军，必须应将", side.name_zh()),
                None,
            ),
            GameStatus::Checkmate { loser } => (
                "checkmate",
                format!(
                    "将死！{}方负，{}方胜",
                    loser.name_zh(),
                    loser.opponent().name_zh()
                ),
                Some(loser),
            ),
            GameStatus::Stalemate { loser } => (
                "stalemate",
                format!(
                    "困毙！{}方无着可走（且未被将军）。中国象棋判困毙方负",
                    loser.name_zh()
                ),
                Some(loser),
            ),
            GameStatus::Draw { reason } => {
                let extra = if pos.is_insufficient_material() {
                    "（双方均无进攻子力）"
                } else {
                    ""
                };
                (
                    "draw",
                    format!("和棋 · {}{extra}", reason.description()),
                    None,
                )
            }
        };
        StatusDto {
            kind: kind.to_string(),
            text,
            over: status.is_over(),
            loser: loser.map(|c| color_word(c).to_string()),
        }
    }
}

#[derive(Serialize)]
pub struct PieceDto {
    pub sq: String,
    pub col: u8,
    pub row: u8,
    pub color: String,
    pub kind: String,
    /// `assets/pieces/<sprite>.svg` 的文件名主干。
    pub sprite: String,
    /// 中文棋子字（无障碍朗读与纯文本降级用）。
    pub glyph: String,
}

/// 当前局面下可选的一个着法。
#[derive(Serialize)]
pub struct MoveOption {
    pub from: String,
    pub to: String,
    pub iccs: String,
    pub notation: String,
    pub capture: bool,
}

/// 已经走过的一步。
#[derive(Serialize, Clone)]
pub struct PlayedMove {
    pub from: String,
    pub to: String,
    pub iccs: String,
    pub notation: String,
    pub capture: bool,
    /// `check` | `capture` | `escape` | `interpose` | … 见 `MoveNature`
    pub nature: String,
    /// 性质的中文说明。
    pub nature_text: String,
    pub side: String,
}

/// 走子成功的响应。
#[derive(Serialize)]
pub struct MoveResponse {
    pub ok: bool,
    pub played: PlayedMove,
    pub state: StateDto,
}

/// 引擎搜索的元信息，用于在界面上展示「引擎在想什么」。
#[derive(Serialize)]
pub struct EngineInfo {
    /// 档位标识，如 `l3`。
    pub level: String,
    /// 档位中文名，如「中级」。
    pub level_label: String,
    /// 完成到的搜索深度。
    pub depth: u8,
    /// 评分（厘兵，走子方视角）。
    pub score: i32,
    /// 搜索节点数。
    pub nodes: u64,
    /// 杀棋距离（步数）；非杀棋为 `null`。
    pub mate_in: Option<i32>,
    /// 是否因时间用尽被提前中断。
    pub stopped: bool,
    /// 实际耗时（毫秒）。
    pub think_ms: u64,
}

/// 引擎落子的响应。
#[derive(Serialize)]
pub struct EngineMoveOutcome {
    pub played: PlayedMove,
    pub info: EngineInfo,
}

/// 一条推荐着法。
#[derive(Serialize)]
pub struct HintItem {
    pub from: String,
    pub to: String,
    pub iccs: String,
    pub notation: String,
    pub score: i32,
    pub capture: bool,
}

/// 走棋提示的响应。
#[derive(Serialize)]
pub struct HintOutcome {
    pub suggestions: Vec<HintItem>,
    pub info: EngineInfo,
}

/// 引擎接口的响应包装。
#[derive(Serialize)]
pub struct EngineMoveResponse {
    pub ok: bool,
    pub engine: EngineMoveOutcome,
    pub state: StateDto,
}

/// 走棋提示的响应包装。
#[derive(Serialize)]
pub struct HintResponse {
    pub ok: bool,
    pub hint: HintOutcome,
}

/// 战法讲解的响应包装。
#[derive(Serialize)]
pub struct CoachResponse {
    pub ok: bool,
    /// 讲解记录。字段见 `xq-coach` 的 `CoachNote`。
    pub note: xq_coach::CoachNote,
    /// 为生成讲解而做的那次搜索的元信息。
    pub info: EngineInfo,
}

/// 悔棋 / 重开的响应。
#[derive(Serialize)]
pub struct StateResponse {
    pub ok: bool,
    pub state: StateDto,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：**走出一步之后**再让引擎应招 / 取提示 / 出讲解，三条链路都必须正常。
    ///
    /// 这里曾经有过一个只在 debug 构建里发作的缺陷：`run_search` 结束后断言
    /// `pos.ply() == 0`，而真正的契约是「还原到**搜索开始时**的层数」。于是只要
    /// 局面不是开局（`ply > 0`），整条链路就 panic —— 前端只看到「响应中途断掉」；
    /// 而 release 版因为 `debug_assert!` 被编译掉，反而一直"看起来是好的"。
    ///
    /// 断言写成 `ply == 0` 的效果是**正常时误报、真出问题时又不查**。所以这个测试
    /// 同时锁住两件事：链路不 panic，且引擎落子后局面确实只前进一步。
    #[test]
    fn engine_and_coach_work_after_a_move() {
        let state = AppState::new();

        // 红方先走一步，制造出 ply > 0 的局面 —— 这正是当初漏测的输入
        let played = state
            .with_game(|game| game.apply_iccs("h2", "e2"))
            .expect("炮二平五应当合法");
        assert_eq!(played.notation, "炮二平五");
        assert_eq!(state.with_game(|game| game.snapshot().ply()), 1);

        // ① 引擎应招 —— 历史崩溃点
        let outcome = state
            .engine_move(Difficulty::L1, 100)
            .expect("引擎应能应招");
        assert_eq!(outcome.played.side, "black");
        // 恰好两步：红方那一步 + 黑方这一步。多了说明搜索没把局面还原干净。
        assert_eq!(state.with_game(|game| game.snapshot().ply()), 2);

        // ② 取提示只搜索、不落子
        state
            .engine_hint(Difficulty::L1, 100, 3)
            .expect("应能给出提示");
        assert_eq!(state.with_game(|game| game.snapshot().ply()), 2);

        // ③ 讲解拿的是「上一着走之前」的局面，层数本来就不为 0 —— 另一个触发点
        let (note, _info) = state
            .coach_last_move(Difficulty::L4, 100)
            .expect("应能生成讲解");
        assert!(!note.headline.is_empty(), "讲解标题不应为空");
        assert_eq!(state.with_game(|game| game.snapshot().ply()), 2);

        // ④ 悔棋把这些都还回去
        assert!(state.with_game(|game| game.undo()));
        assert_eq!(state.with_game(|game| game.snapshot().ply()), 1);
    }

    /// 坐标解析收在会话层，两个宿主共用同一个入口；错误消息必须指明是哪一串坐标。
    #[test]
    fn apply_iccs_rejects_bad_coordinates() {
        let state = AppState::new();

        // 用 `.err()` 而不是 `expect_err()`：DTO 只派生 `Serialize`（它是给前端用的，
        // 不是给人 debug 的），`expect_err` 会对它要求 `Debug`。
        let err = state
            .with_game(|game| game.apply_iccs("h2", "z9"))
            .err()
            .expect("z9 不是合法坐标，应当报错");
        assert!(err.contains("z9"), "错误消息应指明出错的坐标：{err}");

        // 合法坐标但非合法着法：应报「不是合法着法」，而不是崩溃
        let err = state
            .with_game(|game| game.apply_iccs("a0", "a5"))
            .err()
            .expect("车穿不过自己的兵");
        assert!(err.contains("不是当前局面下的合法着法"), "实际：{err}");
    }
}
