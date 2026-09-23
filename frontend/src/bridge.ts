/**
 * 引擎桥接层。
 *
 * # 为什么要抽这一层
 *
 * M1 后半程会把前端装进 Tauri 壳，届时前端**直接调用 Rust command**，
 * 不再经过 HTTP（见 [ADR-002](../../../docs/14-决策记录ADR.md#adr-002)）。
 * 把「怎么跟引擎说话」收在这一个接口后面，换宿主时只需要新增一个实现，
 * 组件层一行都不用改。
 *
 * 目前有两个实现：
 *
 * - `tauriBridge` —— 桌面端，直接调 Rust command（**产品的正式形态**）
 * - `httpBridge`  —— 打到 `xq-bridge` 这个本地 HTTP 服务（开发期的临时通道，
 *                    好处是改前端不用重新编译 Rust）
 *
 * 两者返回的是 `xq-session` 里**同一套 DTO**，所以组件层一行都不用改。
 */

import type {
  CoachResponse,
  DifficultyId,
  EngineMoveResponse,
  HintResponse,
  MoveResponse,
  StateDto,
  StateResponse,
  TimeControlInput,
} from './types'

export interface EngineBridge {
  /** 取当前局面。 */
  state(): Promise<StateDto>
  /** 重开一局；给了 `fen` 则载入该局面，给了 `timeControl` 则启用限时。 */
  newGame(fen?: string, timeControl?: TimeControlInput | null): Promise<StateResponse>
  /** 按坐标走一步。 */
  move(from: string, to: string): Promise<MoveResponse>
  /** 按中文记谱或 ICCS 串走一步。 */
  playText(text: string): Promise<MoveResponse>
  /** 悔一步。 */
  undo(): Promise<StateResponse>
  /** 让引擎走一步。 */
  engineMove(level: DifficultyId, thinkMs: number): Promise<EngineMoveResponse>
  /** 取推荐着法（不落子）。 */
  hint(level: DifficultyId, thinkMs: number, count: number): Promise<HintResponse>
  /** 生成最后一步的战法讲解。 */
  coach(level: DifficultyId, thinkMs: number): Promise<CoachResponse>
}

const BASE = '/api'

async function request<T>(path: string, body?: unknown): Promise<T> {
  let response: Response
  try {
    response = await fetch(`${BASE}${path}`, {
      method: body === undefined ? 'GET' : 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    })
  } catch (cause) {
    throw new Error(
      '连接不上引擎服务。请先启动它：\n  cargo run -p xq-bridge\n' +
        `（原始错误：${cause instanceof Error ? cause.message : String(cause)}）`,
    )
  }

  const text = await response.text()
  let parsed: unknown = null
  if (text.length > 0) {
    try {
      parsed = JSON.parse(text)
    } catch {
      throw new Error(`引擎返回的不是合法 JSON（HTTP ${response.status}）：${text.slice(0, 160)}`)
    }
  }

  if (!response.ok) {
    const message =
      (parsed as { error?: string } | null)?.error ?? `请求失败（HTTP ${response.status}）`
    throw new Error(message)
  }
  return parsed as T
}

export const httpBridge: EngineBridge = {
  state: () => request<StateDto>('/state'),
  newGame: (fen, timeControl) =>
    request<StateResponse>('/new', {
      ...(fen === undefined ? {} : { fen }),
      // 不限时就不发这个字段，让服务端走缺省路径
      ...(timeControl ? { time_control: timeControl } : {}),
    }),
  move: (from, to) => request<MoveResponse>('/move', { from, to }),
  playText: (text) => request<MoveResponse>('/move', { text }),
  // ⚠️ 必须显式传一个 body（哪怕是空对象）：`request` 是以「有没有 body」来决定
  //    HTTP 方法的，不传就发成了 GET，而服务端只接受 POST —— 会拿到一个
  //    「方法不对」的错误。这个坑真的踩过。
  undo: () => request<StateResponse>('/undo', {}),
  engineMove: (level, thinkMs) => request<EngineMoveResponse>('/engine', { level, think_ms: thinkMs }),
  hint: (level, thinkMs, count) => request<HintResponse>('/hint', { level, think_ms: thinkMs, count }),
  coach: (level, thinkMs) => request<CoachResponse>('/coach', { level, think_ms: thinkMs }),
}

// ---------------------------------------------------------------- Tauri 实现

/**
 * Tauri 2 注入的全局对象。
 *
 * 这里用 `window.__TAURI__` 而不是 `@tauri-apps/api` npm 包 —— 因为
 * `tauri.conf.json` 里开了 `withGlobalTauri`，全局对象已经够用，
 * 省掉一个前端依赖和一次 pnpm 安装。
 */
declare global {
  interface Window {
    __TAURI__?: {
      core: { invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> }
    }
  }
}

/**
 * ⚠️ 这里必须 `.bind(...)`：把 `invoke` 从 `__TAURI__.core` 上取下来单独存成
 * 变量后，调用时的接收者就变成 `undefined` 了。当前的 `invoke` 实现恰好不依赖
 * `this`，但这是**实现细节**，不是契约 —— 不加绑定就是在赌上游不改。
 */
const invoke =
  typeof window !== 'undefined'
    ? window.__TAURI__?.core?.invoke.bind(window.__TAURI__.core)
    : undefined

/**
 * 桌面端实现：直接调 Rust，不经过 HTTP。
 *
 * 参数名用 camelCase —— Tauri 2 会把 Rust 侧的 `think_ms` 映射成 JS 侧的
 * `thinkMs`（Rust 用 snake_case、JS 用 camelCase 是 Tauri 的既定约定）。
 */
export const tauriBridge: EngineBridge = {
  state: () => invoke!<StateDto>('engine_state'),
  newGame: (fen, timeControl) =>
    invoke!<StateResponse>('new_game', {
      fen: fen ?? null,
      // Tauri 会把 camelCase 的 JS 参数名映射到 Rust 的 snake_case
      timeControl: timeControl ?? null,
    }),
  move: (from, to) => invoke!<MoveResponse>('make_move', { from, to }),
  playText: (text) => invoke!<MoveResponse>('play_text', { text }),
  undo: () => invoke!<StateResponse>('undo'),
  engineMove: (level, thinkMs) =>
    invoke!<EngineMoveResponse>('engine_move', { level, thinkMs }),
  hint: (level, thinkMs, count) =>
    invoke!<HintResponse>('hint', { level, thinkMs, count }),
  coach: (level, thinkMs) => invoke!<CoachResponse>('coach', { level, thinkMs }),
}

/**
 * 当前使用的桥接实现 —— **按运行环境自动选择**。
 *
 * - 在 Tauri 窗口里 → 直接调 Rust（无 HTTP 层，这是产品的正式形态）
 * - 在普通浏览器里 → 打 HTTP 到 `xq-bridge`（开发期的临时通道）
 *
 * 这就是把「怎么跟引擎说话」收在一个接口后面的收益：切换宿主不需要改任何组件。
 */
export let bridge: EngineBridge = invoke ? tauriBridge : httpBridge

/** 当前运行在桌面端还是浏览器。 */
export const runningInTauri = invoke !== undefined

/** 替换桥接实现（预留给测试）。 */
export function useBridge(next: EngineBridge): void {
  bridge = next
}
