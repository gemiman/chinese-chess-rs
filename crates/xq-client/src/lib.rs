//! # xq-client —— 弈道桌面客户端
//!
//! 前端直接调用 Rust（[ADR-002](../../../docs/14-决策记录ADR.md#adr-002)），
//! **不经过 HTTP**。桥接服务 `xq-bridge` 那层 HTTP 只是开发期的临时通道。
//!
//! # 薄壳设计
//!
//! 本 crate 只做两件事：
//!
//! 1. 建一个窗口，把 `frontend/dist` 装进去；
//! 2. 把 [`xq_session::AppState`] 的能力暴露成一组 Tauri command。
//!
//! **对局逻辑一行都没有** —— 全部在 `xq-session` 里，与 HTTP 桥共用同一份。
//! 所以「同一局棋，用桌面端还是浏览器玩，规则与讲解必然一致」是**结构上保证的**，
//! 不是靠两边小心维护。
//!
//! # 命令与 HTTP 接口一一对应
//!
//! | Tauri command | HTTP | 说明 |
//! |---|---|---|
//! | `engine_state` | `GET /api/state` | 取当前局面 |
//! | `new_game` | `POST /api/new` | 重开 / 载入 FEN |
//! | `make_move` | `POST /api/move` | 按坐标走一步 |
//! | `play_text` | `POST /api/move` | 按记谱走一步 |
//! | `undo` | `POST /api/undo` | 悔棋 |
//! | `engine_move` | `POST /api/engine` | 让引擎走 |
//! | `hint` | `POST /api/hint` | 走棋提示 |
//! | `coach` | `POST /api/coach` | 战法讲解 |
//! | `seek` | `POST /api/seek` | 复盘：跳到第 N 手 |
//! | `resign` | `POST /api/resign` | 认输 |
//! | `settle` | `POST /api/settle` | 当场结算超时 |
//! | `analyze` | `POST /api/analyze` | 赛后深度分析：重算第 N 手 |
//!
//! 返回的都是 `xq-session` 里同一套 DTO，因此前端的 `bridge.ts` 只要换一个实现。
//!
//! # ⚠️ 为什么搜索类的命令必须是 `async fn`
//!
//! Tauri 2 官方文档原文：
//!
//! > Commands without the *async* keyword are executed on the **main thread**
//! > unless defined with `#[tauri::command(async)]`.
//!
//! 主线程同时负责窗口消息循环。`engine_move` / `hint` / `coach` 会跑最长 3 秒的
//! 搜索，**同步实现会把窗口冻住 3 秒**：拖不动、缩不了、点不了关闭。
//! 所以这三个改成 `async fn`，让它们被 `async_runtime::spawn` 丢到线程池上。
//!
//! `async fn` 里带 `State<'_, T>` 这类借用参数本来是不允许的，官方给的解法是
//! **把返回值包进 `Result`** —— 这三个正好都返回 `Result`，天然满足。
//! 其余几个命令耗时在微秒级（走一步、悔一步、取局面），留在主线程反而更快，
//! 不必为了统一而多付一次跨线程调度的开销。

#![deny(unsafe_code)]

use std::sync::Arc;

use tauri::{Manager, State};
use xq_ai::Difficulty;
use xq_session::{
    AppState, CoachResponse, EngineMoveResponse, HintResponse, MoveResponse, StateDto,
    StateResponse, TimeControl, TimeControlInput,
};

/// 启动桌面应用。
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // 启动时把状态挂进去，供各命令取用
            app.manage(Arc::new(AppState::new()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            engine_state,
            new_game,
            make_move,
            play_text,
            undo,
            engine_move,
            hint,
            coach,
            seek,
            resign,
            settle,
            analyze,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}

/// 取当前局面。
#[tauri::command]
fn engine_state(state: State<'_, Arc<AppState>>) -> StateDto {
    state.with_game(|game| game.build_dto())
}

/// 重开一局；给了 `fen` 则载入该局面，给了 `time_control` 则启用限时。
#[tauri::command]
fn new_game(
    state: State<'_, Arc<AppState>>,
    fen: Option<String>,
    time_control: Option<TimeControlInput>,
) -> Result<StateResponse, String> {
    let cfg = time_control
        .map(TimeControl::from)
        .unwrap_or(TimeControl::UNLIMITED);
    // 换局时一并重置引擎，避免上一局的置换表污染新局
    state.reset_engine();
    state.with_game(|game| {
        game.set_time_control(cfg);
        if let Some(fen) = fen {
            game.load_fen(&fen)?;
        } else {
            game.reset();
        }
        Ok(StateResponse {
            ok: true,
            state: game.build_dto(),
        })
    })
}

/// 按坐标走一步。
#[tauri::command]
fn make_move(
    state: State<'_, Arc<AppState>>,
    from: String,
    to: String,
) -> Result<MoveResponse, String> {
    state.with_game(|game| {
        // 与 HTTP 桥走同一个入口：坐标解析在会话层，宿主只负责把字符串递进去。
        let played = game.apply_iccs(&from, &to)?;
        Ok(MoveResponse {
            ok: true,
            played,
            state: game.build_dto(),
        })
    })
}

/// 按中文记谱或 ICCS 串走一步。
#[tauri::command]
fn play_text(state: State<'_, Arc<AppState>>, text: String) -> Result<MoveResponse, String> {
    state.with_game(|game| {
        let played = game.apply_text(&text)?;
        Ok(MoveResponse {
            ok: true,
            played,
            state: game.build_dto(),
        })
    })
}

/// 悔一步。
#[tauri::command]
fn undo(state: State<'_, Arc<AppState>>) -> StateResponse {
    state.with_game(|game| {
        let ok = game.undo();
        StateResponse {
            ok,
            state: game.build_dto(),
        }
    })
}

/// 让引擎走一步。
///
/// `async` 是必须的 —— 见模块头的说明。
#[tauri::command]
async fn engine_move(
    state: State<'_, Arc<AppState>>,
    level: String,
    think_ms: u64,
) -> Result<EngineMoveResponse, String> {
    let level = parse_level(&level)?;
    let outcome = state.engine_move(level, think_ms)?;
    let dto = state.with_game(|game| game.build_dto());
    Ok(EngineMoveResponse {
        ok: true,
        engine: outcome,
        state: dto,
    })
}

/// 取推荐着法（不落子）。
#[tauri::command]
async fn hint(
    state: State<'_, Arc<AppState>>,
    level: String,
    think_ms: u64,
    count: usize,
) -> Result<HintResponse, String> {
    let level = parse_level(&level)?;
    let hint = state.engine_hint(level, think_ms, count)?;
    Ok(HintResponse { ok: true, hint })
}

/// 生成最后一步的战法讲解（内部含一次搜索，所以也是 `async`）。
#[tauri::command]
async fn coach(
    state: State<'_, Arc<AppState>>,
    level: String,
    think_ms: u64,
) -> Result<CoachResponse, String> {
    let level = parse_level(&level)?;
    let (note, info) = state.coach_last_move(level, think_ms)?;
    Ok(CoachResponse {
        ok: true,
        note,
        info,
    })
}

/// 难度标识 → 枚举。与 HTTP 接口接受同一套标识，前端不用区分宿主。
fn parse_level(level: &str) -> Result<Difficulty, String> {
    Difficulty::from_id(level).ok_or_else(|| format!("未知难度档位：{level}"))
}

/// 复盘：把盘面挪到第 `ply` 手之后（`0` = 开局）。
#[tauri::command]
fn seek(state: State<'_, Arc<AppState>>, ply: usize) -> Result<StateResponse, String> {
    let dto = state.seek(ply)?;
    Ok(StateResponse {
        ok: true,
        state: dto,
    })
}

/// 认输。`loser` 是 `"red"` 或 `"black"`。
#[tauri::command]
fn resign(state: State<'_, Arc<AppState>>, loser: String) -> Result<StateResponse, String> {
    let dto = state.resign(&loser)?;
    Ok(StateResponse {
        ok: true,
        state: dto,
    })
}

/// 当场结算超时。前端的倒计时归零时调它。
///
/// 不用 `async`：这里只是读一次时钟、比一下大小，微秒级。慢的是引擎搜索，不是它。
#[tauri::command]
fn settle(state: State<'_, Arc<AppState>>) -> StateResponse {
    StateResponse {
        ok: true,
        state: state.settle_timeout(),
    }
}

/// 赛后深度分析：重算第 `ply` 手的讲解。
///
/// `async` 是必须的 —— 每次调用含一次最长 10 秒的搜索，同步实现会把窗口冻住。
#[tauri::command]
async fn analyze(
    state: State<'_, Arc<AppState>>,
    ply: usize,
    level: String,
    think_ms: u64,
) -> Result<CoachResponse, String> {
    let level = parse_level(&level)?;
    let (note, info) = state.analyze_ply(ply, level, think_ms)?;
    Ok(CoachResponse {
        ok: true,
        note,
        info,
    })
}
