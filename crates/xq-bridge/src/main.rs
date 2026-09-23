//! 本地开发桥接服务。
//!
//! # 它是什么
//!
//! 在 Tauri 客户端（M1 后半程）就绪之前，让浏览器版前端也能用上**真实的 Rust
//! 规则内核**。它只做两件事：
//!
//! 1. 把 `xq-core` 的能力暴露成一组 JSON 接口（`/api/state`、`/api/move` …）；
//! 2. 托管 `frontend/dist` 下的静态文件。
//!
//! # 它不是什么
//!
//! **不是产品组件。** 正式架构里前端由 Tauri 直接调用 Rust（见
//! [ADR-002](../../../docs/14-决策记录ADR.md#adr-002)），不存在这个 HTTP 层。
//! 本服务只是开发期的临时桥，接口形状刻意与将来的 Tauri command 保持一致，
//! 这样前端换宿主时只需要替换 `bridge.ts` 一个文件。
//!
//! # 用法
//!
//! ```bash
//! cargo run -p xq-bridge                 # 服务 127.0.0.1:8848，托管 frontend/dist
//! cargo run -p xq-bridge -- --port 9000  # 换端口
//! cargo run -p xq-bridge -- --dev        # 只提供 API，前端交给 Vite
//! ```

mod http;

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use http::{Request, Response, mime_of};
use xq_ai::Difficulty;
use xq_session::{
    AppState, CoachResponse, EngineMoveResponse, HintResponse, MoveResponse, StateResponse,
};

const DEFAULT_PORT: u16 = 8848;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return;
    }

    let port: u16 = flag_value(&args, "--port")
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let only_api = args.iter().any(|a| a == "--dev");
    let auto_open = args.iter().any(|a| a == "--open");

    let static_root = resolve_static_root();
    let state = Arc::new(AppState::new());

    let addr = format!("127.0.0.1:{port}");
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("无法监听 {addr}：{e}");
            eprintln!("端口可能已被占用，换一个：cargo run -p xq-bridge -- --port 9000");
            std::process::exit(1);
        }
    };

    print_banner(&addr, static_root.as_deref(), only_api);

    // 只有托管了前端才值得开浏览器；--dev 模式下前端在 Vite 那边
    if auto_open && !only_api && static_root.is_some() {
        open_in_browser(&format!("http://{addr}/"));
    }

    let root = static_root.clone();
    let result = http::serve(listener, move |req| {
        route(req, &state, root.as_deref(), only_api)
    });
    if let Err(e) = result {
        eprintln!("服务异常退出：{e}");
        std::process::exit(1);
    }
}

/// 调用系统默认浏览器打开 URL。
///
/// 平台差异用 `cfg!` 分派。失败只打日志不退出 —— 打不开浏览器不该让服务起不来。
fn open_in_browser(url: &str) {
    let spawned = if cfg!(target_os = "windows") {
        // cmd 的 start 把第一个引号参数当作窗口标题，故补一个空标题
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };

    match spawned {
        Ok(_) => println!("已请求系统浏览器打开 {url}"),
        Err(e) => println!("（自动打开浏览器失败：{e} —— 请手动访问 {url}）"),
    }
    println!();
}

fn route(req: &Request, state: &AppState, static_root: Option<&Path>, only_api: bool) -> Response {
    if req.method == "OPTIONS" {
        return Response {
            status: 204,
            content_type: "text/plain".to_string(),
            body: Vec::new(),
            extra_headers: Vec::new(),
        };
    }

    // API 与静态资源严格分流。
    //
    // ⚠️ 这个分流是必需的，不是整理洁癖：如果把 API 路由和静态回退写在同一个
    // match 里，一个方法写错的请求（例如把 POST /api/undo 发成了 GET）会**掉进
    // SPA 回退**，拿到 200 + index.html。前端只会看到「返回的不是合法 JSON」，
    // 完全看不出真正原因是「方法不对」。这个 bug 真的发生过。
    if req.path.starts_with("/api/") {
        return route_api(req, state);
    }

    if only_api {
        return Response::text(
            404,
            "这是 API-only 模式（--dev）。前端静态资源由 Vite 开发服务器提供。\n".to_string(),
        );
    }

    serve_static(&req.path, static_root)
}

fn route_api(req: &Request, state: &AppState) -> Response {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/api/health") => Response::json("{\"ok\":true}".to_string()),

        ("GET", "/api/state") => state.with_game(|game| json_of(&game.build_dto())),

        ("POST", "/api/new") => {
            let body = parse_body(&req.body).ok();
            let fen = body
                .as_ref()
                .and_then(|v| v.get("fen").and_then(|f| f.as_str()).map(str::to_string));
            // 限时配置；缺省即不限时。解析统一在会话层做，两个宿主口径一致。
            let cfg = xq_session::TimeControl::from_json(
                body.as_ref().and_then(|v| v.get("time_control")),
            );
            // 换局时一并重置引擎，避免上一局的置换表污染新局
            state.reset_engine();
            state.with_game(|game| {
                game.set_time_control(cfg);
                if let Some(fen) = fen {
                    if let Err(e) = game.load_fen(&fen) {
                        return json_error(400, &format!("FEN 无法解析：{e}"));
                    }
                } else {
                    game.reset();
                }
                json_of(&StateResponse {
                    ok: true,
                    state: game.build_dto(),
                })
            })
        }

        // 整个处理过程必须在**一次** with_game 内完成 —— Mutex 不可重入，
        // 在闭包里再调 with_game 会死锁。
        ("POST", "/api/move") => {
            let Ok(body) = parse_body(&req.body) else {
                return json_error(400, "请求体不是合法 JSON");
            };
            state.with_game(|game| {
                let outcome = if let Some(text) = body.get("text").and_then(|v| v.as_str()) {
                    game.apply_text(text)
                } else {
                    match (
                        body.get("from").and_then(|v| v.as_str()),
                        body.get("to").and_then(|v| v.as_str()),
                    ) {
                        // 坐标解析交给会话层 —— 适配层不碰领域类型，也就不需要
                        // 依赖 xq-core（这里曾因漏声明依赖而编译失败）。
                        (Some(f), Some(t)) => game.apply_iccs(f, t),
                        _ => Err("需要提供 text，或同时提供 from 与 to".to_string()),
                    }
                };

                match outcome {
                    Ok(played) => {
                        let state_dto = game.build_dto();
                        json_of(&MoveResponse {
                            ok: true,
                            played,
                            state: state_dto,
                        })
                    }
                    Err(e) => json_error(400, &e),
                }
            })
        }

        ("POST", "/api/undo") => state.with_game(|game| {
            let ok = game.undo();
            json_of(&StateResponse {
                ok,
                state: game.build_dto(),
            })
        }),

        // 让引擎走一步。前端「人机对战」模式用这个。
        ("POST", "/api/engine") => {
            let body = parse_body(&req.body).unwrap_or(serde_json::Value::Null);
            let (level, think_ms) = engine_params(&body);
            match state.engine_move(level, think_ms) {
                Ok(outcome) => {
                    let state_dto = state.with_game(|game| game.build_dto());
                    json_of(&EngineMoveResponse {
                        ok: true,
                        engine: outcome,
                        state: state_dto,
                    })
                }
                Err(e) => json_error(400, &e),
            }
        }

        // 只取推荐着法，不落子。前端「走棋提示」用这个。
        ("POST", "/api/hint") => {
            let body = parse_body(&req.body).unwrap_or(serde_json::Value::Null);
            let (level, think_ms) = engine_params(&body);
            let count = body.get("count").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
            match state.engine_hint(level, think_ms, count) {
                Ok(hint) => json_of(&HintResponse { ok: true, hint }),
                Err(e) => json_error(400, &e),
            }
        }

        // 生成最后一步的战法讲解。前端在走子后**异步**请求，不阻塞走棋。
        //
        // 讲解需要一次搜索来拿 `root_moves`（评价定级的唯一依据），所以它比纯本地
        // 模板渲染慢 —— 这正是它必须独立成一个接口、由前端异步调用的原因
        // （docs/05 §6.5 的异步时序：本地讲解先出，增强后到，不打断用户）。
        ("POST", "/api/coach") => {
            let body = parse_body(&req.body).unwrap_or(serde_json::Value::Null);
            // 默认用「高级」档：评价定级的依据是 root_moves 的准确度，
            // 拿弱档位的评分去定级，等级本身就是不准的。
            let level = body
                .get("level")
                .and_then(|v| v.as_str())
                .and_then(Difficulty::from_id)
                .unwrap_or(Difficulty::L4);
            let think_ms = body
                .get("think_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(1_200)
                .clamp(50, 10_000);
            match state.coach_last_move(level, think_ms) {
                Ok((note, info)) => json_of(&CoachResponse {
                    ok: true,
                    note,
                    info,
                }),
                Err(e) => json_error(400, &e),
            }
        }

        // 复盘：把盘面挪到第 N 手之后。前端复盘页前后翻页用这个。
        ("POST", "/api/seek") => {
            let body = parse_body(&req.body).unwrap_or(serde_json::Value::Null);
            let Some(ply) = body.get("ply").and_then(|v| v.as_u64()) else {
                return json_error(400, "需要提供 ply（第几手，0 = 开局）");
            };
            match state.seek(ply as usize) {
                Ok(state_dto) => json_of(&StateResponse {
                    ok: true,
                    state: state_dto,
                }),
                Err(e) => json_error(400, &e),
            }
        }

        // 认输。`loser` 用 DTO 里那套颜色字，前端不必再定义一套。
        ("POST", "/api/resign") => {
            let body = parse_body(&req.body).unwrap_or(serde_json::Value::Null);
            let Some(loser) = body.get("loser").and_then(|v| v.as_str()) else {
                return json_error(400, "需要提供 loser（red 或 black）");
            };
            match state.resign(loser) {
                Ok(state_dto) => json_of(&StateResponse {
                    ok: true,
                    state: state_dto,
                }),
                Err(e) => json_error(400, &e),
            }
        }

        // 当场结算超时。前端的倒计时归零时调它。
        //
        // 不加这个接口的话，超时只有在**有人试着走棋**时才会被发现 ——
        // 玩家盯着一个已经走到 0 的钟，什么都不会发生。所以它必须能被单独调用，
        // 不能只挂在 /api/move 上。
        ("POST", "/api/settle") => {
            let dto = state.settle_timeout();
            json_of(&StateResponse {
                ok: true,
                state: dto,
            })
        }

        // 赛后「深度分析」：重算第 N 手的讲解。前端按手逐个调用，所以要能一次只算一手。
        //
        // ⚠️ 每次调用都含一次搜索（最长 10 秒）。桥接是**一连接一线程**，所以它
        // 不会卡住别的请求；桌面端那边对应的是 `async fn` 命令。两边都别改成同步实现。
        ("POST", "/api/analyze") => {
            let body = parse_body(&req.body).unwrap_or(serde_json::Value::Null);
            let Some(ply) = body.get("ply").and_then(|v| v.as_u64()) else {
                return json_error(400, "需要提供 ply（第几手）");
            };
            let level = body
                .get("level")
                .and_then(|v| v.as_str())
                .and_then(Difficulty::from_id)
                .unwrap_or(Difficulty::L4);
            let think_ms = body
                .get("think_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(1_200)
                .clamp(50, 10_000);
            match state.analyze_ply(ply as usize, level, think_ms) {
                Ok((note, info)) => json_of(&CoachResponse {
                    ok: true,
                    note,
                    info,
                }),
                Err(e) => json_error(400, &e),
            }
        }

        // 其余 /api/* 一律给 JSON 错误 —— 绝不回退到静态资源
        (method, path) => json_error(
            404,
            &format!("未知接口：{method} {path}（可用接口见服务启动日志）"),
        ),
    }
}

/// 从请求体里取 (难度档位, 思考毫秒数)，带默认值与范围保护。
fn engine_params(body: &serde_json::Value) -> (Difficulty, u64) {
    let level = body
        .get("level")
        .and_then(|v| v.as_str())
        .and_then(Difficulty::from_id)
        .unwrap_or(Difficulty::L3);
    let think_ms = body
        .get("think_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(1_200)
        // 上限 30 秒：本地开发工具，防止误填一个巨大的值把界面卡住
        .clamp(50, 30_000);
    (level, think_ms)
}

fn json_of<T: serde::Serialize>(value: &T) -> Response {
    match serde_json::to_string(value) {
        Ok(s) => Response::json(s),
        Err(e) => json_error(500, &format!("序列化失败：{e}")),
    }
}

fn json_error(status: u16, message: &str) -> Response {
    let body = serde_json::json!({ "error": message }).to_string();
    Response {
        status,
        content_type: "application/json; charset=utf-8".to_string(),
        body: body.into_bytes(),
        extra_headers: Vec::new(),
    }
}

fn parse_body(body: &[u8]) -> Result<serde_json::Value, serde_json::Error> {
    if body.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    serde_json::from_slice(body)
}

// ------------------------------------------------------------ 静态资源

fn serve_static(path: &str, root: Option<&Path>) -> Response {
    let Some(root) = root else {
        return Response::text(
            404,
            "前端尚未构建。请先执行：\n  pnpm -C frontend install\n  pnpm -C frontend build\n\
             或用 `--dev` 启动本服务、再跑 `pnpm -C frontend dev`。\n"
                .to_string(),
        );
    };

    let rel = if path == "/" || path.is_empty() {
        "index.html"
    } else {
        path.trim_start_matches('/')
    };

    // 防目录穿越
    if rel.split('/').any(|seg| seg == ".." || seg == ".") || rel.contains('\\') {
        return Response::text(400, "非法路径\n".to_string());
    }

    let full = root.join(rel);
    match std::fs::read(&full) {
        Ok(bytes) => Response::bytes(mime_of(rel), bytes),
        // 单页应用回退：无扩展名的路径当作前端路由，交给 index.html。
        // 带扩展名的资源缺失必须报 404，否则 <img> 会收到 HTML 而报出难以定位的错。
        Err(_) if !rel.contains('.') => match std::fs::read(root.join("index.html")) {
            Ok(bytes) => Response::bytes("text/html; charset=utf-8", bytes),
            Err(_) => Response::text(404, "index.html 不存在，前端可能未构建\n".to_string()),
        },
        Err(_) => Response::text(404, format!("未找到资源：/{rel}\n")),
    }
}

/// 定位前端构建产物目录。
fn resolve_static_root() -> Option<PathBuf> {
    let candidate = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("frontend")
        .join("dist");
    let candidate = candidate.canonicalize().ok()?;
    candidate.join("index.html").is_file().then_some(candidate)
}

// ------------------------------------------------------------ 工具

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let idx = args.iter().position(|a| a == flag)?;
    args.get(idx + 1).cloned()
}

fn print_banner(addr: &str, static_root: Option<&Path>, only_api: bool) {
    println!("弈道 · 本地桥接服务");
    println!("────────────────────────────────────────────");
    println!("  API      http://{addr}/api/state");
    if only_api {
        println!("  界面     由 Vite 提供 → pnpm -C frontend dev（默认 5173）");
    } else {
        match static_root {
            Some(root) => {
                println!("  界面     http://{addr}/        ← 用浏览器打开这个");
                println!("  静态目录 {}", root.display());
                println!("            （下次可以加 --open 让它自动打开浏览器）");
            }
            None => {
                println!("  界面     ✗ frontend/dist 不存在，请先构建前端");
                println!("            pnpm -C frontend install && pnpm -C frontend build");
            }
        }
    }
    println!();
    println!("  接口一览");
    println!("    GET  /api/state                                 取当前局面");
    println!(
        "    POST /api/new     {{\"fen\":\"...\"}}                载入局面（省略 fen 则重开）"
    );
    println!(
        "        可选限时：{{\"time_control\":{{\"base_secs\":600,\"step_secs\":30,\"byoyomi_secs\":30}}}}"
    );
    println!(
        "    POST /api/move    {{\"from\":\"h2\",\"to\":\"e2\"}}        走一步（也可 {{\"text\":\"炮二平五\"}}）"
    );
    println!("    POST /api/undo                                  悔一步");
    println!(
        "    POST /api/engine  {{\"level\":\"l3\",\"think_ms\":1200}}   让引擎走一步（人机对战）"
    );
    println!(
        "    POST /api/hint    {{\"level\":\"l5\",\"count\":3}}        取推荐着法（走棋提示）"
    );
    println!(
        "    POST /api/coach   {{\"level\":\"l4\",\"think_ms\":1200}}  生成最后一步的战法讲解"
    );
    println!(
        "    POST /api/seek    {{\"ply\":12}}                     复盘：把盘面挪到第 12 手之后（0 = 开局）"
    );
    println!("    POST /api/resign  {{\"loser\":\"black\"}}              认输（red / black）");
    println!(
        "    POST /api/settle                                当场结算超时（前端的钟归零时调）"
    );
    println!(
        "    POST /api/analyze {{\"ply\":12,\"level\":\"l4\"}}        赛后深度分析：重算第 12 手的讲解"
    );
    println!("  难度档位 l1 入门 / l2 初级 / l3 中级 / l4 高级 / l5 大师");
    println!();
    println!("  Ctrl+C 停止");
    println!();
}

const USAGE: &str = "\
xq-bridge —— 弈道项目的本地开发桥接服务（非产品组件）

用法：
  cargo run -p xq-bridge [-- --port <端口>] [--open] [--dev]

选项：
  --port <端口>   监听端口，默认 8848
  --open          启动后用系统默认浏览器打开界面
  --dev           仅提供 API（前端交给 Vite 开发服务器）
  --help, -h      显示本帮助
";
