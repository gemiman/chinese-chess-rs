//! 对局棋钟：局时 + 步时 + 读秒。
//!
//! # 计时模型（分段式）
//!
//! 局时未尽时，每步受**步时**限制；局时耗尽后进入**读秒**，读秒每步重置、
//! 不累积。两个阶段互不重叠 —— 步时只在「局时还有」时管用，读秒只在
//! 「局时没了」之后管用。
//!
//! ```text
//!   局时未尽：本步上限 = min(步时, 局时剩余)
//!   局时耗尽：本步上限 = 读秒
//! ```
//!
//! 本步用时超过上限即判负。
//!
//! # 为什么字段名照着 docs/06 起
//!
//! `docs/06-联网对战与实时通信协议.md` 已经把联网版的 `RoomConfig` /
//! `ClockSnapshot` 定好了，且 ADR-014 规定**服务端是时钟的唯一权威**。
//! 本地这版刻意沿用同一套命名与语义，将来接服务端时只剩「谁来记账」的差别，
//! 前端的显示与插值逻辑一行都不用改。
//!
//! # 谁走秒
//!
//! 本模块**不起线程、不设定时器**，而是**惰性结算**：只在「落子」与
//! 「组装 DTO」两个时机按 `Instant` 之差推算当前时间。这样领域层不必持有
//! 后台任务（`xq-core` / `xq-ai` / `xq-coach` 都要求零 IO），前端的秒表
//! 由显示层自己插值走。

use std::time::Instant;

use serde::{Deserialize, Serialize};
use xq_core::Color;

/// 限时配置。三项全为 `0` 视为不限时（此时不会构造 `Clock`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct TimeControl {
    /// 局时（秒）：每方的总用时池。
    pub base_secs: u32,
    /// 步时（秒）：局时未尽时的单步上限。`0` 表示不设单步上限。
    pub step_secs: u32,
    /// 读秒（秒）：局时耗尽后每步的上限（每步重置）。
    pub byoyomi_secs: u32,
}

impl TimeControl {
    /// 不限时。
    pub const UNLIMITED: Self = Self {
        base_secs: 0,
        step_secs: 0,
        byoyomi_secs: 0,
    };

    /// 是否等同于不限时。
    pub const fn is_unlimited(&self) -> bool {
        self.base_secs == 0 && self.step_secs == 0 && self.byoyomi_secs == 0
    }

    /// 从 JSON 解析限时配置。整个对象缺失或为 `null` 即不限时。
    ///
    /// 解析放在会话层而不是各宿主各写一遍 —— 两个宿主对同一份请求必须给出
    /// **一致的解释**，否则又会出现「浏览器里能开局、桌面端报错」这类差异。
    pub fn from_json(value: Option<&serde_json::Value>) -> Self {
        value
            .and_then(|v| serde_json::from_value::<TimeControlInput>(v.clone()).ok())
            .map(TimeControl::from)
            .unwrap_or(TimeControl::UNLIMITED)
    }
}

/// 限时配置的**输入**形状（来自前端的 JSON）。
///
/// 字段全部可选：不传即不启用该维度。单独定义一个输入类型，是因为
/// 输出侧 [`TimeControl`] 的字段是必填的 —— 复用同一个类型会让
/// 「前端少传一个字段」直接反序列化失败。
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct TimeControlInput {
    #[serde(default)]
    pub base_secs: u32,
    #[serde(default)]
    pub step_secs: u32,
    #[serde(default)]
    pub byoyomi_secs: u32,
}

impl From<TimeControlInput> for TimeControl {
    fn from(v: TimeControlInput) -> Self {
        // 上限压到 24 小时，挡住手滑填进来的离谱值
        const MAX: u32 = 86_400;
        Self {
            base_secs: v.base_secs.min(MAX),
            step_secs: v.step_secs.min(MAX),
            byoyomi_secs: v.byoyomi_secs.min(MAX),
        }
    }
}

/// 超时判负。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeout {
    /// 超时的一方，判负。
    pub loser: Color,
}

/// 一方的计时状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SideState {
    /// 局时剩余（毫秒），不会低于 0。
    base_ms: i64,
    /// 是否已进入读秒。
    byoyomi: bool,
}

/// 棋钟快照，送给前端做显示与插值。
#[derive(Clone, Copy, Debug, Serialize)]
pub struct ClockSnapshot {
    pub base_secs: u32,
    pub step_secs: u32,
    pub byoyomi_secs: u32,
    /// 红方局时剩余（毫秒）；已进读秒则为 0。
    pub red_ms: i64,
    pub black_ms: i64,
    pub red_byoyomi: bool,
    pub black_byoyomi: bool,
    /// 当前走子方**本步**还剩多少毫秒，可能为负（已超时）。
    pub step_left_ms: i64,
    /// 当前走子方本步的**上限**（毫秒），即超时判负的阈值。
    ///
    /// 一并下发是为了让前端不必自己推 `min(步时, 局时剩余)`——
    /// 那等于把同一套规则在两边各写一遍，迟早对不上。
    pub step_limit_ms: i64,
}

/// 棋钟。
#[derive(Clone, Debug)]
pub struct Clock {
    cfg: TimeControl,
    red: SideState,
    black: SideState,
    /// 本步开始计时的时刻；`None` 表示当前没有在走秒（终局，或尚未开始）。
    since: Option<Instant>,
    /// 悔棋用的历史：每步落子**前**的双方状态。
    ///
    /// 存在这里而不是 `PlayedMove` 里，是为了不让悔棋用的数据混进
    /// 发给前端的 history —— 那会让 DTO 白白变大一倍。
    undo_stack: Vec<(SideState, SideState)>,
}

impl Clock {
    /// 建一个满血的棋钟。`base_secs == 0` 时直接以读秒开局。
    pub fn new(cfg: TimeControl) -> Self {
        let init = SideState {
            base_ms: cfg.base_secs as i64 * 1000,
            // 没有局时就等于一开始就在读秒
            byoyomi: cfg.base_secs == 0,
        };
        Self {
            cfg,
            red: init,
            black: init,
            since: None,
            undo_stack: Vec::new(),
        }
    }

    /// 开始为当前走子方走秒。已在走秒时是空操作 ——
    /// 组装 DTO 会反复调用它，不能每次都把起点冲掉。
    pub fn start(&mut self, now: Instant) {
        if self.since.is_none() {
            self.since = Some(now);
        }
    }

    /// 停表（终局）。
    pub fn stop(&mut self) {
        self.since = None;
    }

    /// 当前配置。
    pub fn config(&self) -> TimeControl {
        self.cfg
    }

    fn state(&self, side: Color) -> SideState {
        match side {
            Color::Red => self.red,
            Color::Black => self.black,
        }
    }

    fn state_mut(&mut self, side: Color) -> &mut SideState {
        match side {
            Color::Red => &mut self.red,
            Color::Black => &mut self.black,
        }
    }

    /// 本步已用多久。
    fn elapsed_ms(&self, now: Instant) -> i64 {
        self.since
            .map(|t| now.saturating_duration_since(t).as_millis() as i64)
            .unwrap_or(0)
    }

    /// 本步的上限（毫秒）：读秒阶段取读秒，否则取「步时 与 局时剩余」的较小者。
    fn limit_ms(&self, side: Color) -> i64 {
        let s = self.state(side);
        if s.byoyomi {
            return self.cfg.byoyomi_secs as i64 * 1000;
        }
        let base = s.base_ms.max(0);
        if self.cfg.step_secs == 0 {
            base
        } else {
            base.min(self.cfg.step_secs as i64 * 1000)
        }
    }

    /// 归属方本步还剩多少毫秒（可能为负）。
    pub fn step_left_ms(&self, side: Color, now: Instant) -> i64 {
        self.limit_ms(side) - self.elapsed_ms(now)
    }

    /// 现在就到点了吗？
    ///
    /// **只查不改**。不像 [`Clock::settle_move`] 那样扣局时、推进读秒 ——
    /// 那个是「落子那一刻结算」用的，提前调用会把还没走完的这一步直接结掉，
    /// 钟会莫名其妙地跑快。
    ///
    /// 表没在走（终局、或还没开始）时永远返回 `false`。
    pub fn timed_out(&self, side: Color, now: Instant) -> bool {
        self.since.is_some() && self.elapsed_ms(now) > self.limit_ms(side)
    }

    /// 结算一步：扣局时、必要时切读秒；本步超时则返回 `Err`。
    ///
    /// 返回的是本步实际用时（毫秒）。
    pub fn settle_move(&mut self, side: Color, now: Instant) -> Result<i64, Timeout> {
        let used = self.elapsed_ms(now);
        if used > self.limit_ms(side) {
            return Err(Timeout { loser: side });
        }
        self.undo_stack.push((self.red, self.black));
        let s = self.state_mut(side);
        if !s.byoyomi {
            s.base_ms -= used;
            if s.base_ms <= 0 {
                // 局时耗尽 → 下一步起进入读秒
                s.base_ms = 0;
                s.byoyomi = true;
            }
        }
        self.since = None;
        Ok(used)
    }

    /// 悔一步：把棋钟退回该步落子前的样子。
    pub fn undo(&mut self) {
        if let Some((red, black)) = self.undo_stack.pop() {
            self.red = red;
            self.black = black;
            self.since = None;
        }
    }

    /// 组装给前端的快照。
    pub fn snapshot(&self, to_move: Color, now: Instant) -> ClockSnapshot {
        // 归属方的局时正在流逝，快照要给**此刻**的值，而不是上次结算的值 ——
        // 否则前端拿到的数字比真实的大（少算了「上次响应到这次响应」之间那段），
        // 跟同一份快照里的 step_left_ms（是实时算的）对不上。
        let elapsed = self.elapsed_ms(now);
        let mut red_ms = self.red.base_ms;
        let mut black_ms = self.black.base_ms;
        if !self.state(to_move).byoyomi {
            match to_move {
                Color::Red => red_ms = (red_ms - elapsed).max(0),
                Color::Black => black_ms = (black_ms - elapsed).max(0),
            }
        }
        ClockSnapshot {
            base_secs: self.cfg.base_secs,
            step_secs: self.cfg.step_secs,
            byoyomi_secs: self.cfg.byoyomi_secs,
            red_ms,
            black_ms,
            red_byoyomi: self.red.byoyomi,
            black_byoyomi: self.black.byoyomi,
            step_left_ms: self.limit_ms(to_move) - elapsed,
            step_limit_ms: self.limit_ms(to_move),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// 10 分钟局时 + 30 秒步时 + 10 秒读秒
    fn cfg() -> TimeControl {
        TimeControl {
            base_secs: 600,
            step_secs: 30,
            byoyomi_secs: 10,
        }
    }

    /// 走一步：`turn` 是本步用时，返回是否成功。
    fn play(clock: &mut Clock, base: Instant, side: Color, turn: Duration) -> bool {
        clock.start(base);
        clock.settle_move(side, base + turn).is_ok()
    }

    #[test]
    fn step_time_caps_a_single_move() {
        let mut clock = Clock::new(cfg());
        let t = Instant::now();
        // 局时还剩 10 分钟，但单步超过步时 30 秒 → 直接判负
        assert!(!play(&mut clock, t, Color::Red, Duration::from_secs(31)));
    }

    /// `timed_out` 必须**只查不改**，而且要能查出真正的到点。
    ///
    /// 这两件事是一体的：如果它图省事调了 `settle_move`，查一次就把这一步结掉了
    /// （`since` 变 `None`），于是**第二次起永远查不出超时** —— 前端每 100 毫秒查一次，
    /// 结果就是「钟走到 0 了，但怎么都不判负」。而它同时还会每次扣一遍局时，
    /// 钟会随着「查得越勤」跑得越快。
    ///
    /// 用合成的时间点（`t0 + Duration`）而不是真的 sleep，测试才既确定又快。
    #[test]
    fn timed_out_only_reads_and_detects_expiry() {
        let one_second_step = TimeControl {
            base_secs: 60,
            step_secs: 1,
            byoyomi_secs: 0,
        };
        let t0 = Instant::now();

        // 表还没开始走 → 永远不判超时
        let idle = Clock::new(one_second_step);
        assert!(!idle.timed_out(Color::Red, t0 + Duration::from_secs(99)));

        let mut clock = Clock::new(one_second_step);
        clock.start(t0);
        assert!(!clock.timed_out(Color::Red, t0), "刚开始不该到点");
        // 连着查：查到一半的状态不能被它改掉
        assert!(!clock.timed_out(Color::Red, t0 + Duration::from_millis(500)));
        assert!(!clock.timed_out(Color::Red, t0 + Duration::from_millis(999)));
        assert!(
            clock.timed_out(Color::Red, t0 + Duration::from_millis(1_001)),
            "过了步时就该查到"
        );

        // 停表之后不再判 —— 终局了就不该再被翻转结果
        clock.stop();
        assert!(!clock.timed_out(Color::Red, t0 + Duration::from_secs(99)));
    }

    #[test]
    fn step_time_does_not_touch_base_time_when_within_limit() {
        let mut clock = Clock::new(cfg());
        let t = Instant::now();
        assert!(play(&mut clock, t, Color::Red, Duration::from_secs(12)));
        // 12 秒从局时里扣掉
        assert_eq!(clock.red.base_ms, 600_000 - 12_000);
        assert_eq!(clock.black.base_ms, 600_000);
        assert!(!clock.red.byoyomi);
    }

    #[test]
    fn base_time_running_out_switches_to_byoyomi() {
        let mut clock = Clock::new(cfg());
        let t = Instant::now();
        // 局时只剩 4 秒时走一步用满 4 秒 → 局时归零，下一步进入读秒
        clock.settle_move(Color::Red, t).ok();
        clock.red.base_ms = 4_000;
        clock.start(t);
        assert!(
            clock
                .settle_move(Color::Red, t + Duration::from_secs(4))
                .is_ok()
        );
        assert_eq!(clock.red.base_ms, 0);
        assert!(clock.red.byoyomi, "局时耗尽后应进入读秒");
    }

    #[test]
    fn byoyomi_resets_every_move() {
        let mut clock = Clock::new(cfg());
        let t = Instant::now();
        clock.red.byoyomi = true;
        clock.red.base_ms = 0;

        // 连续两步都用满读秒上限，都应当成立 —— 读秒每步重置，不累积
        clock.start(t);
        assert!(
            clock
                .settle_move(Color::Red, t + Duration::from_secs(10))
                .is_ok()
        );
        let t2 = t + Duration::from_secs(10);
        clock.start(t2);
        assert!(
            clock
                .settle_move(Color::Red, t2 + Duration::from_secs(10))
                .is_ok()
        );
    }

    #[test]
    fn byoyomi_timeout_loses() {
        let mut clock = Clock::new(cfg());
        let t = Instant::now();
        clock.red.byoyomi = true;
        clock.start(t);
        // 读秒 10 秒，用了 11 秒 → 判负
        let err = clock
            .settle_move(Color::Red, t + Duration::from_secs(11))
            .unwrap_err();
        assert_eq!(err.loser, Color::Red);
    }

    #[test]
    fn base_time_shrinks_the_step_limit() {
        let mut clock = Clock::new(cfg());
        let t = Instant::now();
        // 局时只剩 2 秒时，本步上限被压到 2 秒（而不是步时的 30 秒）
        clock.red.base_ms = 2_000;
        clock.start(t);
        assert!(
            clock
                .settle_move(Color::Red, t + Duration::from_millis(2_500))
                .is_err()
        );
    }

    #[test]
    fn undo_restores_the_clock() {
        let mut clock = Clock::new(cfg());
        let t = Instant::now();
        assert!(play(&mut clock, t, Color::Red, Duration::from_secs(20)));
        assert_eq!(clock.red.base_ms, 580_000);
        clock.undo();
        assert_eq!(clock.red.base_ms, 600_000, "悔棋应把局时退回去");
    }

    #[test]
    fn snapshot_reports_both_sides() {
        let mut clock = Clock::new(cfg());
        let t = Instant::now();
        assert!(play(&mut clock, t, Color::Red, Duration::from_secs(7)));
        // 走完这步就轮到黑方，时钟从此刻起走（真实链路里 apply_move 会做这件事）
        let t1 = t + Duration::from_secs(7);
        clock.start(t1);
        let snap = clock.snapshot(Color::Black, t1 + Duration::from_secs(3));
        assert_eq!(snap.base_secs, 600);
        // 红方已经结算过，7 秒从局时里扣掉
        assert_eq!(snap.red_ms, 593_000);
        // 黑方的局时**正在流逝**，快照要给此刻的值：600 秒 − 已走 3 秒
        assert_eq!(snap.black_ms, 597_000);
        // 归属方（黑）本步还剩：步时 30 秒 − 已走 3 秒
        assert_eq!(snap.step_left_ms, 27_000);
        assert_eq!(snap.step_limit_ms, 30_000);
    }

    #[test]
    fn snapshot_does_not_run_down_byoyomi() {
        let mut clock = Clock::new(cfg());
        let t = Instant::now();
        clock.black.byoyomi = true;
        clock.black.base_ms = 0;
        clock.start(t);
        // 读秒阶段局时不再流逝（每步重置），所以黑方局时仍是 0、红方不受影响
        let snap = clock.snapshot(Color::Black, t + Duration::from_secs(5));
        assert_eq!(snap.black_ms, 0);
        assert_eq!(snap.red_ms, 600_000);
    }

    #[test]
    fn zero_base_starts_in_byoyomi() {
        let clock = Clock::new(TimeControl {
            base_secs: 0,
            step_secs: 0,
            byoyomi_secs: 20,
        });
        assert!(clock.red.byoyomi, "没设局时就等于一开始就在读秒");
        assert_eq!(clock.limit_ms(Color::Red), 20_000);
    }

    #[test]
    fn json_parsing_defaults_to_unlimited() {
        assert!(TimeControl::from_json(None).is_unlimited());
        let v = serde_json::json!({});
        assert!(TimeControl::from_json(Some(&v)).is_unlimited());
        let v = serde_json::json!({ "base_secs": 300, "step_secs": 15 });
        let cfg = TimeControl::from_json(Some(&v));
        assert_eq!(cfg.base_secs, 300);
        assert_eq!(cfg.step_secs, 15);
        assert_eq!(cfg.byoyomi_secs, 0);
    }
}
