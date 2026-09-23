# 11 · 工程规范与 CI/CD

| 项 | 值 |
|---|---|
| 项目代号 | 弈道 / chinese-chess-rs |
| 文档版本 | v1.0 |
| 状态 | 设计基线（Design Baseline） |
| 编制日期 | 2026-09-23 |
| 目标读者 | 全体开发、测试、运维 |
| 关联文档 | [README §3 技术栈](../README.md) · [docs/01 §8 验收标准](01-需求规格说明书.md) · [docs/02 §3 Crate 划分](02-系统架构设计.md) · [docs/02 §10 错误处理](02-系统架构设计.md) · [ADR-005 零 IO](14-决策记录ADR.md#adr-005) · [docs/12 测试策略](12-测试策略与质量保障.md) |

> **本文的定位**：把「怎么写得一致」「怎么提交」「CI 必须卡住什么」「怎么发版」四件事写成**可执行、可判定**的规范。
> 文中所有**阈值、耗时预算、并发数**均为**设计初始值**，标注为「估算」或「待实测」者不得作为验收结论。

---

## 1. 代码规范

### 1.1 统一原则

| 原则 | 落地手段 |
|---|---|
| 格式不存在争议 | `cargo fmt` + `prettier` 全自动，CI 强制 `--check`，评审**不提格式意见** |
| Lint 分级明确 | 见 §1.3。`deny` 级违反即 CI 失败；`warn` 级不阻塞但需在 PR 中说明 |
| 领域层更严 | `xq-core` / `xq-ai` / `xq-coach` 三个 crate 额外启用「禁止 unwrap / panic / 读时钟 / 读文件」的严格 lint |
| 规范可被机器验证 | 每条规范尽量落到 lint、脚本或测试；无法自动化的才写进 Checklist |

### 1.2 `rustfmt.toml`（可直接使用）

放在 workspace 根目录。**只使用 stable 选项**，保证 `cargo fmt --check` 在 Rust 1.98.1 上零警告执行。

```toml
# ============================================================
# rustfmt.toml — 弈道 / chinese-chess-rs
# 约束：仅使用 stable 选项，CI 中执行 `cargo fmt --all -- --check`。
#       nightly-only 选项见文末注释块，不作为门禁。
# ============================================================

edition        = "2024"
style_edition  = "2024"

required_version = "1.98.1"

# ---- 行宽与缩进 ----
max_width    = 100
hard_tabs    = false
tab_spaces   = 4
indent_style = "Block"
newline_style = "Auto"

# ---- 小结构体/调用链的换行启发式（显式写出，避免版本默认值漂移）----
use_small_heuristics           = "Max"
fn_call_width                  = 80
attr_fn_like_width             = 80
struct_lit_width               = 60
struct_variant_width           = 40
array_width                    = 80
chain_width                    = 80
single_line_if_else_max_width  = 60
single_line_let_else_max_width = 60

# ---- import / 模块顺序 ----
reorder_imports = true
reorder_modules = true

# ---- 代码整洁 ----
merge_derives           = true
use_field_init_shorthand = true
use_try_shorthand        = true
remove_nested_parens     = true
condense_wildcard_suffixes = true
force_explicit_abi       = true

# ---- 文档注释 ----
normalize_doc_attributes = true
wrap_comments            = true
comment_width            = 100
format_code_in_doc_comments = true

# ============================================================
# 以下选项需要 nightly 工具链，**不作为 CI 门禁**。
# 本地可选执行：`cargo +nightly fmt --all`
# 引入前需团队评审，因为它会大范围重排 import 分组。
# ============================================================
# imports_granularity = "Module"
# group_imports       = "StdExternalCrate"
# reorder_impl_items  = true
# error_on_line_overflow = true
```

> **注**：`wrap_comments` / `comment_width` / `format_code_in_doc_comments` / `normalize_doc_attributes` 在部分工具链上仍属 nightly 选项。**若 CI 出现 "unstable features are only available in nightly channel" 警告**，直接从配置中删除这几项即可——rustfmt 对未识别的 unstable 选项只会警告，不会报错。这是刻意的容错设计：**规范不能让构建挂掉**。

**团队约定**：
- 缩进 4 空格，不用 Tab；
- 行宽 100（比 rustfmt 默认 100 一致，比常见的 120 更严格，有利于 side-by-side diff）；
- `use` 语句按 rustfmt 默认排序，**不手工分组**（避免格式化抖动）。

### 1.3 `clippy.toml`（可直接使用）

```toml
# ============================================================
# clippy.toml — 弈道 / chinese-chess-rs
# 位置：workspace 根目录（clippy 会自动向父目录查找）
# ============================================================

# 与 rust-toolchain.toml 保持一致；clippy 据此避免建议新版本才有的 API
msrv = "1.98"

# ---- 复杂度阈值 ----
cognitive-complexity-threshold = 20      # 默认 25，本项目收紧
too-many-arguments-threshold    = 8      # 引擎内部函数参数较多（见下方说明）
type-complexity-threshold       = 300
too-many-lines-threshold        = 300    # movegen / eval 的查表函数可能较长
single-char-binding-names-threshold = 6

# ---- 错误类型体积 ----
large-error-threshold = 256              # xq-core 错误枚举含字符串时避免意外膨胀

# ---- 测试代码放宽（关键：让 unwrap_used 只在生产代码生效）----
allow-unwrap-in-tests        = true
allow-expect-in-tests        = true
allow-panic-in-tests         = true
allow-dbg-in-tests           = true
allow-print-in-tests         = true
allow-indexing-slicing-in-tests = true

# ---- 术语白名单（避免 doc_markdown 误报）----
doc-valid-idents = [
    "ICCS", "FEN", "NPS", "ELO", "Zobrist", "Alpha", "Beta", "PVS",
    "MVV", "LVA", "PST", "NNUE", "WASM", "Tauri", "SeaORM", "Axum",
    "Tokio", "PostgreSQL", "Redis", "WebView", "SQLx", "MCTS", "PV",
]

# ---- 禁用名（避免无意义的短名）----
disallowed-names = ["foo", "bar", "baz", "tmp", "data2", "t2", "aaa"]

# ---- 禁用方法：全局层面只禁用"明确有替代品"的 ----
disallowed-methods = [
    { path = "std::env::set_var",        reason = "多线程环境下不安全；配置一律走 config crate 注入" },
    { path = "std::process::exit",       reason = "库层禁止；进程退出只能由 main / xq-server 顶层的受控关停路径执行" },
    { path = "std::thread::sleep",       reason = "业务代码禁止阻塞式 sleep；等待请用 tokio::time 或测试专用辅助函数" },
    { path = "rand::thread_rng",         reason = "领域层必须使用显式种子的 StdRng（ADR-005）；应用层随机请走注入的 RNG" },
]

# ---- 例外与禁用 ----
avoid-breaking-exported-api = false
upper-case-acronyms-aggressive = true
```

> **为什么 `too-many-arguments-threshold = 8`**：走法生成与评估函数天然带较多参数（`pos`、`color`、`depth`、`alpha`、`beta`…）。默认阈值 7 会在热点路径上产生大量无意义的 `#[allow]`，反而降低可读性。8 是「容忍热点、约束业务代码」的折中。

**`disallowed-methods` 与 ADR-005 的关系**：`rand::thread_rng` 与 `std::process::exit` 的禁用是**全局**的，与 [ADR-005](14-决策记录ADR.md#adr-005) 的精神一致。`SystemTime::now` 无法全局禁用（`xq-server` / `xq-client` 合法需要它），改用 **§5.3 的源码断言脚本**按 crate 定向拦截。

### 1.4 Clippy lint 分级策略

lint 配置写在 workspace 根的 `Cargo.toml`，成员 crate 通过 `[lints] workspace = true` 继承。

```toml
# ---- 根 Cargo.toml ----

[workspace.lints.rust]
unsafe_code      = "forbid"   # README: 零 unsafe
unused_must_use  = "deny"
missing_debug_implementations = "warn"
missing_docs     = "warn"
rust_2018_idioms = { level = "deny", priority = -1 }
unused_qualifications = "warn"

[workspace.lints.clippy]
# ---------- 基线：全开 ----------
all      = { level = "warn",   priority = -1 }
pedantic = { level = "warn",   priority = -1 }

# ---------- deny：违反即 CI 失败 ----------
dbg_macro              = "deny"
todo                   = "deny"   # 用 unimplemented!("见 docs/13 M3") 的形式显式登记
unimplemented          = "deny"
print_stdout           = "deny"   # 一律用 tracing
print_stderr           = "deny"
float_cmp              = "deny"   # 浮点比较必须走 approx / 显式容差
float_cmp_const        = "deny"
panic_in_result_fn     = "deny"
mem_forget             = "deny"
exit                   = "deny"
unwrap_used            = "deny"
expect_used            = "deny"
panic                  = "deny"
indexing_slicing       = "warn"   # 热点路径（board[90]）需要 allow，热点处显式标注
exhaustive_enums       = "deny"   # 公开枚举必须 #[non_exhaustive] 或明确"永不变"
exhaustive_structs     = "deny"
cargo_common_metadata  = "deny"
multiple_crate_versions = "warn"
redundant_clone        = "warn"
wildcard_imports       = "deny"   # 禁止 `use foo::*`，prelude 模块除外
pattern_type_mismatch  = "warn"

# ---------- allow：经评审的例外，必须写明理由 ----------
module_name_repetitions   = "allow" # `board::Board` 这类命名在领域模型中更自然
missing_errors_doc        = "allow" # 错误语义统一写在 docs/02 §10，不逐个函数重复
missing_panics_doc        = "allow"
must_use_candidate        = "allow" # 与 missing_docs 一起会造成大量噪音
similar_names             = "allow" # alpha / beta / alpha_beta / beta_cut 是本域术语
cast_possible_truncation  = "allow" # 位棋盘与 u8/u16 索引转换密集
cast_sign_loss            = "allow"
cast_precision_loss       = "allow"
cast_possible_wrap        = "allow"
struct_excessive_bools    = "allow" # 难度配置天然是多布尔
```

**领域层额外收紧**（在 `crates/xq-core/src/lib.rs`、`xq-ai/src/lib.rs`、`xq-coach/src/lib.rs` 顶部声明）：

```rust
// xq-core / xq-ai / xq-coach 三者的 lib.rs 顶部
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![deny(clippy::indexing_slicing)]          // 领域层禁止裸索引，用 get() + 明确错误
#![deny(clippy::print_stdout, clippy::print_stderr)]
#![deny(clippy::disallowed_methods)]
#![warn(clippy::arithmetic_side_effects)]   // 盘面索引计算必须显式处理溢出
```

> **为什么领域层要 `deny(indexing_slicing)`**：`Position` 的 `squares[90]` 索引是热点，但**越界即 panic**。领域层的「永不 panic」契约（见 [docs/05 §7.1](05-战法讲解引擎设计.md)：`analyze` 必须永不失败）要求索引必须走 `get()` 或经证明安全的 `const` 边界。热点处的确需要裸索引时，用**局部 `#[allow(clippy::indexing_slicing)]` + 一行注释说明不变式**，让例外在 diff 中可见。

### 1.5 前端 ESLint + Prettier

**技术栈**：ESLint 9（Flat Config）+ typescript-eslint + Prettier + Vitest。

`frontend/eslint.config.js`：

```js
// @ts-check
import js from '@eslint/js';
import tseslint from 'typescript-eslint';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';
import prettier from 'eslint-config-prettier';

export default tseslint.config(
  {
    ignores: [
      'dist/**',
      'coverage/**',
      'node_modules/**',
      'src-tauri/**',            // Rust 侧由 cargo clippy 负责
      'src/api/generated/**',    // 由 xq-protocol 生成的类型声明
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommendedTypeChecked,
  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      ecmaVersion: 2024,
      sourceType: 'module',
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,

      // ---- 类型纪律 ----
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/no-unsafe-assignment': 'error',
      '@typescript-eslint/no-unsafe-member-access': 'error',
      '@typescript-eslint/consistent-type-imports': ['error', { prefer: 'type-imports' }],
      '@typescript-eslint/no-floating-promises': 'error',
      '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_', varsIgnorePattern: '^_' }],
      '@typescript-eslint/switch-exhaustiveness-check': 'error',

      // ---- 工程纪律 ----
      eqeqeq: ['error', 'always'],
      'no-console': ['warn', { allow: ['warn', 'error'] }],
      'prefer-const': 'error',
      'no-restricted-imports': ['error', {
        paths: [
          { name: 'lodash', message: '请用原生实现或引入具体子模块，避免整包体积' },
        ],
      }],

      // ---- 性能敏感：棋盘渲染相关（见 docs/08 §16）----
      'react-refresh/only-export-components': ['warn', { allowConstantExport: true }],
    },
  },
  {
    // 测试文件放宽
    files: ['**/*.test.{ts,tsx}', '**/__tests__/**'],
    rules: {
      '@typescript-eslint/no-explicit-any': 'off',
      'no-console': 'off',
    },
  },
  prettier, // 必须在最后：关闭所有与 Prettier 冲突的格式规则
);
```

`frontend/.prettierrc.json`：

```json
{
  "printWidth": 100,
  "tabWidth": 2,
  "useTabs": false,
  "semi": true,
  "singleQuote": true,
  "jsxSingleQuote": false,
  "trailingComma": "all",
  "bracketSpacing": true,
  "arrowParens": "always",
  "endOfLine": "lf",
  "overrides": [
    { "files": "*.md", "options": { "proseWrap": "preserve" } }
  ]
}
```

`frontend/.prettierignore`：

```
dist
coverage
node_modules
src-tauri
pnpm-lock.yaml
src/api/generated
```

**核心要点**：

| 要点 | 理由 |
|---|---|
| `printWidth = 100` 与 Rust 侧对齐 | 双语种切换时认知一致 |
| `eslint-config-prettier` 放最后 | 彻底消除「ESLint 说换行、Prettier 说合并」的循环报告 |
| `no-explicit-any` 设为 `error` | 前端直接消费 `xq-protocol` 契约，类型松了就等于契约丢了（[docs/02 §2.1 不变量 I-2](02-系统架构设计.md)） |
| `switch-exhaustiveness-check` 置 error | 协议帧是**判别联合**，漏处理一帧必须编译期/检查期暴露 |
| 坐标换算模块单独设更严规则 | 见 §2.4，棋盘坐标换算是最易出错的前端逻辑（[ADR-008](14-决策记录ADR.md#adr-008)） |

---

## 2. 命名与代码组织规范

### 2.1 模块划分原则

crate 划分见 [docs/02 §3](02-系统架构设计.md)，此处只规定**crate 内部**的模块组织：

| 原则 | 说明 |
|---|---|
| **P1 按领域概念分文件，不按技术分层** | `position.rs` / `movegen.rs` / `repetition.rs` 好于 `models.rs` / `utils.rs` / `helpers.rs` |
| **P2 一个模块一个核心类型** | 模块名 = 核心类型名（`board::Board`、`tt::TranspositionTable`） |
| **P3 `lib.rs` 只做门面** | 只包含 re-export、crate 级文档、lint 属性。**不含业务逻辑**，长度不超过 120 行 |
| **P4 禁止 `utils.rs` / `common.rs` / `helpers.rs`** | 这类文件是「没有归属」的信号。若确实有共享函数，按**它服务的主概念**命名（如 `square.rs`、`zobrist.rs`） |
| **P5 测试与实现同文件** | 单元测试放 `#[cfg(test)] mod tests`；集成测试放 `tests/`；基准放 `benches/` |
| **P6 `pub(crate)` 优先** | 能不进公开 API 的一律 `pub(crate)`。公开面越大，向后兼容负担越重 |
| **P7 feature gate 显式** | `serde`、`llm` 等可选能力用 feature 控制，且在 `Cargo.toml` 中写注释说明"谁需要它" |

**目录参考**（以 `xq-core` 为例，与 [docs/03 §10](03-规则引擎与领域模型.md) 一致）：

```
crates/xq-core/
├── src/
│   ├── lib.rs            # 门面 + prelude + lint 属性（≤120 行）
│   ├── color.rs
│   ├── piece.rs
│   ├── square.rs
│   ├── position.rs
│   ├── movegen.rs
│   ├── rules.rs
│   ├── repetition.rs
│   ├── nature.rs
│   ├── zobrist.rs
│   ├── fen.rs
│   └── notation.rs
├── tests/
│   ├── perft.rs
│   ├── golden_positions.rs   # 对拍题库驱动
│   └── properties.rs         # proptest
├── benches/
│   └── core_bench.rs
└── testdata/
    └── positions/            # 对拍题库（见 docs/12 §3）
```

### 2.2 命名约定

| 类别 | 规范 | 正例 | 反例 |
|---|---|---|---|
| crate | `xq-` 前缀 + 领域词（kebab-case） | `xq-core`、`xq-coach` | `core`、`chess_core` |
| 模块 / 文件 | snake_case，单数名词 | `movegen.rs`、`notation.rs` | `moves.rs`、`notations.rs` |
| 类型 / trait | UpperCamelCase，**不用前缀后缀堆砌** | `Position`、`Searcher`、`StopSignal` | `PositionData`、`IPosition` |
| 枚举变体 | UpperCamelCase，语义完整 | `GameStatus::Checkmate { loser }` | `GameStatus::CM` |
| 函数 / 方法 | snake_case，动词开头，**读起来像一句话** | `is_in_check(color)`、`legal_moves()` | `check()`、`get_moves()` |
| 布尔函数 | `is_` / `has_` / `can_` 前缀 | `has_legal_move()` | `legal_move()` |
| 常量 | SCREAMING_SNAKE_CASE | `BOARD_SIZE`、`MATE_SCORE` | `boardSize` |
| 单位后缀 | 标量必须带单位后缀 | `timeout_ms`、`halfmove_clock` | `timeout`、`clock` |
| 坐标 | 用 `sq` 表示 index、`(col, row)` 表示坐标 | `from_sq: u8`、`col: u8` | `from: u8`（歧义） |
| 颜色/方 | 一律用 `Color::Red` / `Color::Black`，**禁止 `white/black`** | 符合中国象棋术语 | `White` |
| 测试函数 | `should_<expected>_when_<condition>` | `should_reject_move_when_horse_leg_blocked` | `test_horse()` |

> **`Color` 命名说明**：中国象棋用**红/黑**而非白/黑。代码、协议、数据库统一用 `Red` / `Black`，FEN 中的 `w` / `b` 只在 `fen.rs` 边界处转换（[docs/03 §8.2](03-规则引擎与领域模型.md)）。这是容易出现「文档说红、代码说白」漂移的地方，必须在评审中留意。

### 2.3 公开 API 设计原则

| 原则 | 说明 | 落地检查 |
|---|---|---|
| **API-1 最小公开面** | 除 [docs/03 §10](03-规则引擎与领域模型.md) / [docs/02 §3.2](02-系统架构设计.md) 明列的 API 概览外，一律 `pub(crate)` | `cargo public-api` 快照对比（见 §5.4） |
| **API-2 类型不可变优先** | 构造后不变的类型优先；需要变更的用 `&mut self` 显式方法 | 评审 |
| **API-3 返回类型表达意图** | 「可能失败」→ `Result`；「可能不存在」→ `Option`；「永不失败」→ 直接返回值 | 见 §2.5 |
| **API-4 公开枚举 `#[non_exhaustive]`** | 跨 crate 的公开枚举（`GameStatus`、错误码）必须加，避免下游 `match` 被新变体破坏 | clippy `exhaustive_enums` |
| **API-5 参数用新类型而非 `bool`/`u8`** | `Move(u16)` 而非 `u16`；`Depth(u8)` 优于裸 `u8` | 评审 + `clippy::struct_excessive_bools` |
| **API-6 无隐藏全局状态** | 所有可变状态通过 `&mut self` 显式传递（`Searcher` 是独占对象，见 [docs/02 §3.2](02-系统架构设计.md)） | 评审 |
| **API-7 零 IO（领域层）** | 见 §5.3 的自动断言 | CI 脚本 |
| **API-8 性能承诺要写在文档里** | 分配型 API（`legal_moves() -> Vec<Move>`）与无分配 API（`has_legal_move()`）**必须都在文档中标明**其适用场景 | doc 注释审查 |

### 2.4 前端代码组织

| 目录 | 职责 | 禁止 |
|---|---|---|
| `src/components/board/` | 棋盘渲染、棋子、高亮层 | 禁止直接调 `invoke`，只接收 props |
| `src/components/coach/` | 讲解卡片、评价徽标 | 禁止做战术判定，只渲染 |
| `src/stores/` | Zustand 状态 | 禁止直接访问 DOM |
| `src/api/` | Tauri `invoke` 封装 + 类型 | **禁止在组件内直接 `invoke`**，必须经此层 |
| `src/coords/` | ICCS ↔ 像素坐标换算 | **必须纯函数、零副作用、100% 覆盖**（[ADR-008](14-决策记录ADR.md#adr-008) 后果条款） |
| `src/features/` | 按业务场景聚合（对局 / 房间 / 复盘） | 禁止跨 feature 直接 import，经 `src/api` 或 store |

**坐标换算的硬约束**：`src/coords/` 下所有导出函数必须满足
① 纯函数（无 `Date`、无随机、无全局状态）；
② 单测覆盖 90 格全部（见 [docs/12 §2.7](12-测试策略与质量保障.md)）；
③ 往返不变式 `pxToIccs(iccsToPx(sq)) === sq` 用属性测试锁定。

### 2.5 错误处理规范

严格遵循 [docs/02 §10](02-系统架构设计.md) 的三层分工：

| 层 | crate / 模块 | 类型 | 规则 |
|---|---|---|---|
| 领域层 | `xq-core` / `xq-ai` / `xq-coach` | `thiserror` 具名枚举 | 调用方需要**匹配**错误变体做分支。**禁止** `anyhow` |
| 应用层 | `xq-server` / `xq-client` 的 handler、service | `anyhow::Error` + `.context()` | 只需向上传播与记录。上下文必须包含可定位信息（`room_id` / `game_id`） |
| 对外边界 | HTTP / WS | `ApiError` / `ErrorFrame` | 稳定错误码契约，见 [docs/10 §4](10-API接口规范.md) |

**具体规则**：

```rust
// ✅ 领域层：具名错误枚举，变体语义完整
#[derive(Debug, thiserror::Error)]
pub enum IllegalMove {
    #[error("起点无棋子: {from:?}")]
    EmptySource { from: Square },
    #[error("目标被己方棋子占据: {to:?}")]
    OwnPieceAtTarget { to: Square },
    #[error("着法导致己方将帅被将军")]
    LeavesKingInCheck,
    #[error("该着法不满足 {kind:?} 的走法规则")]
    PieceRuleViolation { kind: PieceKind },
}

// ✅ 应用层：anyhow + 上下文
let snapshot = room
    .snapshot()
    .await
    .with_context(|| format!("获取房间快照失败 room_id={room_id}"))?;

// ❌ 禁止：对外错误消息泄露内部细节（docs/02 §10 原则 1）
return Err(ApiError::internal(format!("sqlx error: {e:?}")));   // 泄露 SQL
return Err(ApiError::internal(format!("{e:?}")));                // 泄露文件路径/堆栈
```

**错误转换约定**：

| 方向 | 方式 | 要求 |
|---|---|---|
| 领域 → 应用 | `impl From<IllegalMove> for anyhow::Error`（自动） | 保留原始类型，便于 `downcast_ref` |
| 应用 → 对外 | 显式 `match` + 映射表 | **不允许** `impl From<E> for ApiError` 的兜底实现（会掩盖未分类错误） |
| 对外 → 客户端 | 错误码字符串 + 人话文案 | 未知错误码优雅降级为通用提示，**不崩溃**（[docs/02 §10 原则 3](02-系统架构设计.md)） |

**错误消息红线**（由 CI 断言强制，见 §5.3）：

1. 对外错误消息**不得**包含：SQL 片段、文件路径、`Debug` 格式的结构体、堆栈；
2. 服务端日志**必须**包含完整上下文与 `request_id`；
3. 未知错误码在客户端必须降级而非崩溃。

### 2.6 注释与文档注释规范

| 位置 | 要求 |
|---|---|
| crate 级 `//!` | 每个 crate 的 `lib.rs` 必须有：职责一句话、依赖约束、对应的设计文档链接（`[docs/03](...)`） |
| 公开项 `///` | `missing_docs = "warn"` 已强制。文档注释必须包含：**做什么**、**不做什么**、**复杂度/性能承诺**（若有）、**失败条件**（若有） |
| 私有项 `//` | 只在**逻辑不自明**处写。写「为什么」，不写「是什么」 |
| 规则实现 | 每一处非平凡的规则判定**必须**标注来源：`// 见 docs/03 §4.2 马：蹩腿点必须是空格` |
| 性能优化 | 每个「为性能牺牲可读性」的地方必须写明：优化什么、实测收益、可回退路径 |
| `#[allow(...)]` | **必须**同行或上一行写明理由：`#[allow(clippy::indexing_slicing)] // sq 由 movegen 保证 < 90，见 tests/bounds.rs` |
| TODO | 格式固定：`// TODO(M3): <做什么> —— 见 docs/13 §M3`。**必须**带里程碑标记，`clippy::todo` 为 deny 时改为 `unimplemented!("TODO(M3): ...")` 并注明 |

**文档注释的"契约"要求**：凡是在设计文档中声明过的契约（如「`analyze` 永不失败」「`root_moves` 必定包含全部根着法」），**必须**在对应函数的 doc 注释中重申一遍，并给出测试文件名。原因：读代码的人不一定读文档，但契约不能只活在文档里。

```rust
/// 生成讲解。
///
/// # 契约
/// **本方法永不失败。** 任何内部异常都会降级为兜底讲解，而不是返回 `Err`。
/// 这是为了保证对局流程不因讲解失败而中断（见 docs/05 §7.1）。
/// 该契约由 `tests/never_fail.rs` 用 10000 次随机输入锁定。
pub fn analyze(&self, ...) -> CoachNote
```

---

## 3. 提交规范

### 3.1 格式

采用 [Conventional Commits 1.0.0](https://www.conventionalcommits.org/)：

```
<type>(<scope>): <subject>

[body]

[footer]
```

| 部分 | 要求 |
|---|---|
| `type` | 见 §3.2 类型清单，**必填**，小写 |
| `scope` | 见 §3.3，**选填**但强烈建议；一个提交只允许**一个** scope |
| `subject` | 祈使句、中文、不超过 50 字符、**句末不加句号** |
| `body` | 与「为什么」相关；每行 ≤ 72 字符；可为空 |
| `footer` | `Refs: #123` / `Closes: #123` / `BREAKING CHANGE:` |

### 3.2 类型清单

| type | 用途 | 会进入 CHANGELOG |
|---|---|---|
| `feat` | 新功能 | ✅ Feature |
| `fix` | 缺陷修复 | ✅ Bug Fixes |
| `perf` | 性能优化（不改行为） | ✅ Performance |
| `refactor` | 重构（不改行为、不改性能承诺） | ❌ |
| `test` | 增删改测试 | ❌ |
| `docs` | 仅文档 | ❌ |
| `build` | 构建系统、依赖、工具链 | ✅ 视内容 |
| `ci` | CI 配置与脚本 | ❌ |
| `chore` | 杂项（版本号、`Cargo.lock` 手工维护等） | ❌ |
| `style` | 纯格式（`cargo fmt` 批量结果） | ❌ |
| `revert` | 回滚 | ✅ Reverts |

### 3.3 作用域约定

scope 一律用**短横线小写**，取自 crate 名或子模块名：

| scope | 覆盖范围 |
|---|---|
| `core` | `xq-core`（棋盘 / 走法 / 规则 / 记谱 / FEN） |
| `ai` | `xq-ai`（搜索 / 评估 / 开局库） |
| `coach` | `xq-coach`（战术识别 / 模板 / LLM 抽象） |
| `protocol` | `xq-protocol`（DTO / WS 帧 / 错误码） |
| `server` | `xq-server` 顶层 |
| `room` / `ws` / `matchmaking` / `store` / `coach-svc` | `xq-server` 子模块 |
| `client` | `xq-client`（Tauri Rust 侧） |
| `ui` | `frontend/` 组件与样式 |
| `coords` | `frontend/src/coords/`（坐标换算） |
| `deps` | 依赖升降级 |
| `toolchain` | Rust / Node / 工具链版本 |
| `assets` | `assets/` 素材 |
| `migrations` | `migrations/` 数据库迁移 |
| `docs` | `docs/` 只用 `docs` 作 type，不用 scope |

### 3.4 示例

```
feat(core): 实现马的蹩腿点检测

蹩腿点只需为空，与目标点是什么棋子无关。这一点与「塞象眼」
的判定一致，但与「炮架」语义不同（炮架可以是任意方棋子）。

影响：legal_moves() 在初始局面的输出由 45 修正为 44，
与 docs/03 §11.2 的 perft(1) 手工推导一致。

测试：tests/perft.rs::perft_depth_1 == 44
Refs: #42
```

```
fix(room): 断线判负后未清理待决的求和请求

对局进入 Finished 后 pending_draw 仍留在房间快照中，
导致重连的客户端会收到已失效的求和提示。

Closes: #187
```

```
perf(core): is_attacked 改为反向探测，bench 提升 42%

依据 docs/03 §4.4 的反向探测模型，跳过正向生成。
criterion 基线：core/attack_detection 由 173 ns → 100 ns。
```

**破坏性变更**：

```
feat(protocol)!: MoveApplied 增加 rules_version 字段

BREAKING CHANGE: 帧结构变更，客户端需同步升级。
服务端保留 1 个版本的向后兼容窗口（docs/06 §13）。
Refs: ADR-003
```

### 3.5 提交粒度与自动化

| 规则 | 说明 |
|---|---|
| 一提交一件事 | 一个提交能独立回滚、独立通过 CI |
| `cargo fmt` 单独提交 | 批量格式化不要与逻辑改动混在一起（否则 diff 无法评审） |
| `Cargo.lock` 随 `build(deps)` 提交 | **必须**一并提交，保证可复现构建 |
| 迁移文件独立提交 | `migrations/` 的变更单独成一个提交，便于回溯 |
| CI 校验 | `commitlint` 在 PR 上校验标题与所有提交（见 §5.5） |

**本地钩子**（可选但推荐）：

```bash
# .githooks/commit-msg   （启用：git config core.hooksPath .githooks）
#!/usr/bin/env sh
set -e
msg_file="$1"
pattern='^(feat|fix|perf|refactor|test|docs|build|ci|chore|style|revert)(\([a-z0-9-]+\))?!?: .{1,50}$'
head -n1 "$msg_file" | grep -Eq "$pattern" || {
  echo "提交标题不符合 Conventional Commits 规范（见 docs/11 §3）"
  echo "示例: feat(core): 实现马的蹩腿点检测"
  exit 1
}
```

---

## 4. 分支策略与 PR 流程

### 4.1 分支模型

采用**简化主干开发**（Trunk-based with short-lived branches），不引入 develop/release 长期分支：

| 分支 | 用途 | 生命周期 | 保护 |
|---|---|---|---|
| `main` | 唯一主干，**始终可发布** | 永久 | ✅ 禁止直推、禁止 force push、必须通过全部门禁 |
| `feat/<issue>-<slug>` | 功能开发 | ≤ 5 天，超过需拆分 | — |
| `fix/<issue>-<slug>` | 缺陷修复 | ≤ 2 天 | — |
| `perf/<issue>-<slug>` | 性能优化（必须附 bench 对比） | ≤ 3 天 | — |
| `chore/<slug>` | 依赖、工具链、配置 | ≤ 1 天 | — |
| `release/<version>` | **仅在需要冻结做三端回归时**临时创建 | 从打 tag 到发布完成 | — |
| `hotfix/<version>` | 生产紧急修复，从 tag 拉出，修完合回 `main` | ≤ 1 天 | — |

**分支命名规则**：`<type>/<issue编号>-<kebab-case-摘要>`，例：`feat/128-room-reconnect`。

**为什么不用 GitFlow**：本项目单人/小团队节奏、发布频率低于每周、无并行的多版本维护需求。GitFlow 的 `develop` / `release` / `hotfix` 三支线在这里只会增加合并噪音。**当出现"需要同时维护两个已发布版本"的需求时，再引入 `release/*` 长期分支并新增 ADR。**

### 4.2 合并策略

| 场景 | 策略 | 理由 |
|---|---|---|
| `feat` / `fix` → `main` | **Squash and Merge** | 主干历史保持"一 PR 一提交"，便于 `git bisect` 与 `revert` |
| Squash 后的提交标题 | 取 PR 标题（已由 commitlint 校验） | 保持 Conventional Commits 一致 |
| `release` → `main` | Merge Commit | 保留发布边界 |
| `hotfix` → `main` | Merge Commit + `revert` 记录 | 便于审计 |

**合并前置条件（全部满足才可点 Merge）**：

1. 至少 **1 名** Reviewer 批准；涉及以下任一领域需 **2 名**：`xq-core` 规则改动、`migrations/`、CI 配置、鉴权与安全相关；
2. 所有 CI 阶段绿（见 §5.6 质量门禁）；
3. 分支与 `main` 无冲突；
4. 无 `TODO`/`FIXME` 新增（除非带里程碑标记）；
5. 覆盖率未下降（或下降已在 PR 说明中论证合理）。

### 4.3 PR 检查清单（作者自检）

作者在开 PR 前**逐条自检**，Reviewer 据此复核：

```markdown
<!-- .github/pull_request_template.md -->

## 变更摘要
<!-- 一句话说明这个 PR 做了什么 -->

## 关联
- Issue: #
- 设计文档章节: docs/xx §x.x
- ADR（若有决策）: ADR-0xx

## 检查清单（作者自检）

### 正确性
- [ ] 新增/修改的规则行为有对应的测试用例，且测试能**因这个改动而失败**
- [ ] 涉及 perft / 对拍题库的改动，已重新运行并附上结果
- [ ] 涉及 `xq-ai` 的改动，已附 criterion 前后对比（`perf` 类 PR 必须附）
- [ ] 未引入 `unwrap()` / `expect()` / `panic!()`（领域层绝对禁止；应用层需有注释说明不可达性）
- [ ] 浮点比较使用显式容差，未用 `==`

### 接口与契约
- [ ] 公开 API 变更已在 PR 描述中列出，并说明是否破坏兼容
- [ ] `xq-protocol` 变更已确认两端同步（[docs/02 §2.1 不变量 I-2](02-系统架构设计.md)）
- [ ] 数据库变更通过 `migrations/` 落地，且**回滚路径已考虑**
- [ ] 公开枚举新增变体时已加 `#[non_exhaustive]` 或确认无需

### 约束
- [ ] 领域层（core/ai/coach）未引入任何 IO 依赖或时钟调用（ADR-005）
- [ ] 未在客户端引入任何服务端密钥（NFR-S.09）
- [ ] 日志中未记录密码 / token / 完整手机号（§9.3）
- [ ] 新增配置项已加入 `config/default.toml` 与 `.env.example`，且**默认值安全**

### 工程
- [ ] `cargo fmt` / `cargo clippy` / `pnpm lint` 本地已通过
- [ ] 提交标题符合 Conventional Commits
- [ ] 新增文档链接使用**相对路径**
- [ ] 无遗留 `dbg!` / `console.log` / 注释掉的死代码

## 风险与回滚
- 风险:
- 回滚方式:（revert commit / 关闭 feature flag / 回滚迁移）
```

### 4.4 评审要点（Reviewer 视角）

Reviewer **按优先级从高到低**看，避免把注意力花在格式上（格式由工具管）：

| 优先级 | 关注点 | 典型问题 |
|---|---|---|
| **P0 正确性** | 规则实现是否与 [docs/03](03-规则引擎与领域模型.md) 一致？边界是否覆盖？ | 蹩腿点判成"必须空且非己方"（多算了条件） |
| **P0 约束违反** | 领域层是否引入 IO？是否绕过了服务端权威？ | 在 `xq-coach` 里直接 `reqwest::get` |
| **P0 安全** | 是否有注入面？是否信任了客户端输入？ | 直接把客户端 FEN 当权威局面 |
| **P1 契约** | 公开 API / 协议 / 错误码是否变了？向后兼容吗？ | 悄悄给 `MoveDto` 加必填字段 |
| **P1 测试质量** | 测试是否能在实现错误时真的失败？ | `assert!(result.is_ok() \|\| result.is_err())` 这类空断言 |
| **P1 性能** | 热点路径是否引入了分配 / 克隆？ | 在搜索内层 `Vec::new()` |
| **P2 可读性** | 命名是否表达意图？复杂逻辑是否有注释？ | `let x = calc(a, b, c);` |
| **P2 文档** | 契约是否写进 doc 注释？设计文档是否需要同步更新？ | 改了分档阈值但没回写 docs/01 §4.3 |
| **P3 格式** | — | **不提**（工具负责） |

> **评审的原则**：**不要求作者改风格，要求作者改证据。** 与其说"这里可能有问题"，不如说"请补一个覆盖 XX 的用例"。
>
> **文档同步是硬要求**：若 PR 改变了设计文档中声明的任何**数值、阈值、契约**，必须在同一个 PR 内更新对应文档。文档与实现的漂移是这类项目最大的隐性债务。

---

## 5. CI 流水线设计

### 5.1 选型与运行环境

**选型：GitHub Actions**。理由：三端矩阵原生支持（windows-latest / ubuntu-latest / macos-latest 免费）、与 Release / 制品管理天然集成、Tauri 官方 `tauri-action` 可用。

> 若后续迁移到自建 GitLab CI，本文的阶段划分与脚本片段**完全可复用**（所有检查项都是纯 shell 命令，不依赖 Actions 专有语法），只需重写 job 编排部分。

| 项 | 值 |
|---|---|
| 触发 | `pull_request` → `main`：跑 §5.2 全部**阻塞**阶段<br>`push` → `main`：跑全部 + 保存基准<br>`push` tag `v*`：跑发布流水线（§6）<br>`schedule`（每日 03:00 UTC）：跑重型回归（对拍题库全量 + 自对弈 1000 局） |
| 并发控制 | `concurrency: { group: ci-${{ github.ref }}, cancel-in-progress: true }` |
| 缓存 | Cargo registry / target（`Swatinem/rust-cache`）、pnpm store |
| 超时 | 单 job 默认 30 分钟；重型回归 job 60 分钟 |

### 5.2 阶段总览

> **耗时预估为估算值**（首次冷缓存可能翻倍），仅用于排期参考，**非承诺**。标注 `S` 的阶段为阻塞阶段，失败即 PR 不可合并。

| # | 阶段 | 阻塞 | 职责 | 关键命令 | 预估耗时 |
|---|---|---|---|---|---|
| 1 | `fmt` | S | 格式检查（Rust + 前端） | `cargo fmt --all -- --check`、`pnpm prettier --check .` | ~1 min |
| 2 | `lint-rust` | S | Clippy 全 workspace，`-D warnings` | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ~5 min |
| 3 | `dep-guard` | S | **ADR-005 依赖断言 + 源码 IO 断言 + 错误消息红线** | §5.3 脚本 | ~2 min |
| 4 | `build` | S | 编译全 workspace（debug） | `cargo build --workspace --all-targets` | ~6 min |
| 5 | `test-unit` | S | 单元测试（含 doc test） | `cargo nextest run --workspace --lib` + `cargo test --doc` | ~4 min |
| 6 | `test-integration` | S | 集成测试（需 PostgreSQL + Redis service） | `cargo nextest run --workspace --test '*'` | ~8 min |
| 7 | `api-surface` | S | 公开 API 快照对比（防止无意破坏兼容） | `cargo public-api --diff` | ~3 min |
| 8 | `frontend` | S | 前端 lint / 类型 / 测试 / 构建 | `pnpm lint`、`tsc --noEmit`、`vitest run --coverage`、`pnpm build` | ~5 min |
| 9 | `coverage` | S | 覆盖率统计与门禁（仅在 `ubuntu-latest`） | `cargo llvm-cov` + §5.6 门禁脚本 | ~10 min |
| 10 | `cross-check` | S | 非 Tauri crate 的三端交叉编译检查 | `cargo check -p xq-core -p xq-ai -p xq-coach -p xq-protocol --target <T>` | ~6 min |
| 11 | `bench-guard` | 非阻塞/可配置 | 性能基准回归检测（PR 中与 `main` 基线对比） | `cargo bench` + `critcmp` | ~12 min |
| 12 | `heavy-regression` | 仅 nightly | perft 全深度 + 对拍题库全量 + 自对弈 1000 局 + 对拍题库外挂库 | `cargo test --release -p xq-core --features heavy` | ~40 min |
| 13 | `tauri-smoke` | 非阻塞（M1 后转阻塞） | 三端构建冒烟（不上传制品） | `pnpm tauri build --debug` | ~15 min/端 |
| 14 | `release` | 仅 tag | 三端正式打包 + 签名 + 发布（§6） | 见 §6.4 | ~30 min |

**阶段依赖关系**：

```mermaid
graph LR
    FMT[1 fmt] --> LINT[2 lint-rust]
    FMT --> DEP[3 dep-guard]
    DEP --> BUILD[4 build]
    LINT --> BUILD
    BUILD --> TU[5 test-unit]
    BUILD --> TI[6 test-integration]
    BUILD --> API[7 api-surface]
    BUILD --> XC[10 cross-check]
    TU --> COV[9 coverage]
    TI --> COV
    COV --> GATE{质量门禁}
    API --> GATE
    XC --> GATE
    FE[8 frontend] --> GATE
    GATE --> BENCH[11 bench-guard]
    GATE --> HEAVY[12 heavy-regression]
    GATE --> TS[13 tauri-smoke]
```

### 5.3 阶段 3：依赖断言与约束检查脚本（**核心**）

> 本阶段是 [ADR-005](14-决策记录ADR.md#adr-005) 与 [docs/02 §2.1 不变量 I-1](02-系统架构设计.md) 的**唯一强制执行点**。没有这条流水线，零 IO 约束会在几个月内被无声破坏。

`scripts/ci/assert_no_io_deps.sh`：

```bash
#!/usr/bin/env bash
# ============================================================
# ADR-005 依赖断言
# 校验 xq-core / xq-ai / xq-coach 三个领域层 crate 的依赖树中
# 不出现任何 IO / 运行时 / 存储 crate。
#
# 用法： bash scripts/ci/assert_no_io_deps.sh
# 退出码：0 = 通过；1 = 违反约束
# ============================================================
set -euo pipefail

# 被禁依赖（ADR-005 明确列举）
BANNED_NORMAL=(
  tokio axum sqlx sea-orm sea-orm-migration
  redis deadpool-redis reqwest hyper
)

# 领域层三个 crate
DOMAIN_CRATES=(xq-core xq-ai xq-coach)

# 领域层禁止出现在源码中的 std 用法（ADR-005）
BANNED_SOURCE_PATTERNS=(
  'std::fs'
  'std::net'
  'std::time::SystemTime'
  'std::process::'
  'rand::thread_rng'
  'rand::random'
)

FAILED=0

log()  { printf '%s\n' "$*"; }
fail() { printf '\033[31m%s\033[0m\n' "$*"; FAILED=1; }
ok()   { printf '\033[32m%s\033[0m\n' "$*"; }

# ------------------------------------------------------------
# 检查 1：cargo tree —— 生产依赖与构建依赖中不得出现被禁 crate
# ------------------------------------------------------------
for crate in "${DOMAIN_CRATES[@]}"; do
  log "==> [cargo tree] ${crate}（edges: normal,build）"

  # --prefix none : 每个包一行，便于精确 grep
  # --edges normal,build : 只查生产与构建依赖
  tree="$(cargo tree -p "${crate}" --edges normal,build --prefix none --locked)"

  for banned in "${BANNED_NORMAL[@]}"; do
    # 匹配形如 "tokio v1.53.1" 的整行，避免误伤 "tokio-util" 之类
    if printf '%s\n' "${tree}" | grep -Eq "^${banned} v[0-9]"; then
      fail "  [违反 ADR-005] ${crate} 依赖了 ${banned}"
      printf '%s\n' "${tree}" | grep -E "^${banned} v[0-9]" | sed 's/^/      /'
    fi
  done

  # 额外：检查是否有任何 crate 的 build.rs 依赖了 cc/openssl 之类（提示性，不失败）
  if printf '%s\n' "${tree}" | grep -Eq '^openssl-sys v'; then
    log "  [提示] ${crate} 依赖了 openssl-sys，请确认是否有纯 Rust 替代"
  fi

  ok "  ${crate} 依赖树干净"
done

# ------------------------------------------------------------
# 检查 2：dev-dependencies 中的被禁 crate —— 仅告警，不失败
# 理由：领域层测试允许使用 tokio 做并发压力测试，但应尽量避免。
#      默认告警可让评审注意到这个信号。
# ------------------------------------------------------------
for crate in "${DOMAIN_CRATES[@]}"; do
  dev_tree="$(cargo tree -p "${crate}" --edges dev --prefix none --locked || true)"
  for banned in "${BANNED_NORMAL[@]}"; do
    if printf '%s\n' "${dev_tree}" | grep -Eq "^${banned} v[0-9]"; then
      log "  [告警] ${crate} 的 dev-dependencies 含 ${banned}（允许但需评审）"
    fi
  done
done

# ------------------------------------------------------------
# 检查 3：源码级断言 —— 领域层不得直接使用 IO / 系统时钟 / 线程随机
# ------------------------------------------------------------
for crate in "${DOMAIN_CRATES[@]}"; do
  src_dir="crates/${crate}/src"
  [ -d "${src_dir}" ] || { log "==> 跳过 ${crate}（src 目录不存在）"; continue; }

  log "==> [源码断言] ${src_dir}"

  for pattern in "${BANNED_SOURCE_PATTERNS[@]}"; do
    # grep 会匹配注释中的示例，故先剔除 /// 与 // 开头的行
    hits="$(
      grep -rn --include='*.rs' -F "${pattern}" "${src_dir}" \
        | grep -v '^\s*[^:]*:[0-9]*:\s*//' \
        || true
    )"
    if [ -n "${hits}" ]; then
      fail "  [违反 ADR-005] ${crate} 源码中出现禁用模式: ${pattern}"
      printf '%s\n' "${hits}" | sed 's/^/      /'
    fi
  done

  ok "  ${crate} 源码无 IO / 时钟 / 线程随机"
done

# ------------------------------------------------------------
# 检查 4：错误消息红线（docs/02 §10 原则 1、docs/10 C-07）
# ------------------------------------------------------------
log "==> [错误消息红线] 对外错误不得拼接内部细节"

# 在对外边界（ApiError / ErrorFrame 构造处）禁止出现 Debug 格式化拼接
for file in crates/xq-server/src/error.rs crates/xq-protocol/src/error.rs; do
  [ -f "${file}" ] || continue
  if grep -nE 'format!\("[^"]*\{:\?\}' "${file}" | grep -v '^\s*//' >/dev/null 2>&1; then
    fail "  [违反 docs/02 §10] ${file} 中对外错误消息使用了 {e:?} 调试格式"
  fi
done
ok "  错误消息红线检查通过"

# ------------------------------------------------------------
echo "------------------------------------------------------------"
if [ "${FAILED}" -ne 0 ]; then
  fail "依赖 / 约束断言失败 —— 构建终止（ADR-005）"
  echo "请检查：① 是否为了便利在领域层引入了 IO；② 是否能用 trait 抽象替代（见 ADR-005 例外与处理）"
  exit 1
fi
ok "全部约束断言通过（ADR-005 / docs/02 §2.1 / docs/02 §10）"
```

**同时需要一条反向断言**：`xq-core` 的依赖数量必须保持极简（只有 `serde` / `thiserror` / 必要工具 crate）。这防止"渐进式依赖膨胀"。

```bash
# scripts/ci/assert_dep_budget.sh
set -euo pipefail
# xq-core 的直接依赖上限（设计初始值，调整需在 PR 中说明理由）
LIMIT_CORE=8
LIMIT_AI=10
LIMIT_COACH=12

check() {
  local crate="$1" limit="$2"
  local n
  n="$(cargo tree -p "${crate}" --depth 1 --prefix none --edges normal --locked \
        | grep -c ' v' || true)"
  echo "${crate} 直接依赖数 = ${n}（上限 ${limit}）"
  if [ "${n}" -gt "${limit}" ]; then
    echo "[失败] ${crate} 直接依赖数超上限，请评审是否必要"
    exit 1
  fi
}
check xq-core  "${LIMIT_CORE}"
check xq-ai    "${LIMIT_AI}"
check xq-coach "${LIMIT_COACH}"
echo "依赖预算检查通过"
```

> 上表三个上限值是**设计初始值**，需在 M0 完成、依赖定型后依据实际值校准并回写本文档。

**CI 中的用法片段**：

```yaml
dep-guard:
  name: 阶段3 依赖与约束断言
  runs-on: ubuntu-latest
  needs: [fmt]
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
      with:
        toolchain: 1.98.1
    - uses: Swatinem/rust-cache@v2
    - name: ADR-005 依赖断言
      run: bash scripts/ci/assert_no_io_deps.sh
    - name: 依赖预算检查
      run: bash scripts/ci/assert_dep_budget.sh
```

### 5.4 阶段 7：公开 API 快照

`xq-protocol` 是两端唯一契约（[docs/02 §2.1 不变量 I-2](02-系统架构设计.md)），它的意外变更必须被 CI 拦下。

```bash
# scripts/ci/check_public_api.sh
set -euo pipefail

if ! command -v cargo-public-api >/dev/null 2>&1; then
  echo "安装 cargo-public-api ..."
  cargo install cargo-public-api --locked
fi

mkdir -p target/api-baseline

# 基线存于仓库（api-baseline/），PR 中对比
for crate in xq-core xq-ai xq-coach xq-protocol; do
  echo "==> ${crate}"
  cargo public-api -p "${crate}" --simplified > "target/api-${crate}.txt"

  baseline="api-baseline/${crate}.txt"
  if [ -f "${baseline}" ]; then
    if ! diff -u "${baseline}" "target/api-${crate}.txt"; then
      echo ""
      echo "[失败] ${crate} 的公开 API 发生变化。"
      echo "若这是有意变更：① 在 PR 描述中说明破坏性；② 更新基线："
      echo "  cp target/api-${crate}.txt ${baseline}"
      exit 1
    fi
  else
    echo "[提示] ${crate} 无基线，本次将建立基线（请提交 api-baseline/${crate}.txt）"
  fi
done
echo "公开 API 快照一致"
```

### 5.5 阶段 8：前端流水线

```yaml
frontend:
  name: 阶段8 前端 lint / 类型 / 测试 / 构建
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: pnpm/action-setup@v4
      with:
        version: 10
    - uses: actions/setup-node@v4
      with:
        node-version: 22
        cache: pnpm
        cache-dependency-path: frontend/pnpm-lock.yaml
    - name: 安装依赖
      working-directory: frontend
      run: pnpm install --frozen-lockfile
    - name: 格式检查
      working-directory: frontend
      run: pnpm exec prettier --check .
    - name: ESLint
      working-directory: frontend
      run: pnpm exec eslint . --max-warnings 0
    - name: 类型检查
      working-directory: frontend
      run: pnpm exec tsc --noEmit
    - name: 单元测试 + 覆盖率
      working-directory: frontend
      run: pnpm exec vitest run --coverage
    - name: 构建
      working-directory: frontend
      run: pnpm build
    - name: 体积预算检查
      working-directory: frontend
      run: node scripts/check-bundle-size.mjs
```

**体积预算**（NFR-P.09 安装包 ≤ 30 MB 的上游约束，**设计初始值，待实测校准**）：

| 项 | 预算 | 说明 |
|---|---|---|
| `dist/` 总大小（gzip 后） | ≤ 2.5 MB | 超限则需分包或裁剪依赖 |
| 首屏 JS（gzip） | ≤ 400 KB | 对应 NFR-P.08 冷启动 ≤ 2 s |
| 单个 chunk（gzip） | ≤ 700 KB | 防止一次性巨大解析 |
| 素材总量 | ≤ 200 KB | SVG 素材，避免位图 |

**commitlint 校验**（同 job 内）：

```yaml
    - name: 校验提交规范
      if: github.event_name == 'pull_request'
      uses: wagoid/commitlint-github-action@v6
      with:
        configFile: .commitlintrc.json
```

### 5.6 阶段 9：覆盖率与质量门禁

**工具**：`cargo-llvm-cov`（Rust）、`vitest --coverage`（前端）。

覆盖率目标来自 [NFR-M.02](01-需求规格说明书.md)（核心 crate 单元测试覆盖率 ≥ 85%，规则模块 ≥ 95%）。

```bash
# scripts/ci/coverage_gate.sh
set -euo pipefail

REPORT="target/coverage/summary.json"
mkdir -p target/coverage

# 生成覆盖率（lcov + json summary），排除测试自身与生成代码
cargo llvm-cov --workspace --all-features \
  --ignore-filename-regex '(tests|benches|target)/' \
  --json --output-path "${REPORT}" \
  --lcov --output-path target/coverage/lcov.info

# 门槛定义（来源：docs/01 NFR-M.02；规则模块为逐文件门槛）
declare -A FILE_GATE=(
  ["crates/xq-core/src/movegen.rs"]=95
  ["crates/xq-core/src/rules.rs"]=95
  ["crates/xq-core/src/repetition.rs"]=95
  ["crates/xq-core/src/fen.rs"]=95
  ["crates/xq-core/src/notation.rs"]=95
  ["crates/xq-coach/src/tactics.rs"]=90
  ["crates/xq-ai/src/search.rs"]=85
)

# crate 级门槛（全覆盖率）
CRATE_GATE_CORE=85
CRATE_GATE_AI=85
CRATE_GATE_COACH=85
CRATE_GATE_PROTOCOL=90

python3 scripts/ci/check_coverage.py \
  --report "${REPORT}" \
  --crate-gate "xq-core=${CRATE_GATE_CORE}" \
  --crate-gate "xq-ai=${CRATE_GATE_AI}" \
  --crate-gate "xq-coach=${CRATE_GATE_COACH}" \
  --crate-gate "xq-protocol=${CRATE_GATE_PROTOCOL}" \
  --file-gate "$(for k in "${!FILE_GATE[@]}"; do echo "${k}=${FILE_GATE[$k]}"; done | paste -sd, -)"
```

`scripts/ci/check_coverage.py` 的核心逻辑（**要点：门禁要"可判定"，不能是模糊的"大致达标"**）：

```python
#!/usr/bin/env python3
"""覆盖率门禁。读取 cargo-llvm-cov 的 JSON summary，逐项比对阈值。"""
import argparse
import json
import sys


def parse_kv(items: list[str]) -> dict[str, float]:
    out: dict[str, float] = {}
    for item in items:
        if not item:
            continue
        key, _, val = item.partition("=")
        out[key.strip()] = float(val)
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", required=True)
    ap.add_argument("--crate-gate", action="append", default=[])
    ap.add_argument("--file-gate", default="")
    args = ap.parse_args()

    with open(args.report, encoding="utf-8") as fh:
        data = json.load(fh)

    # 汇总每个文件的覆盖率，并聚合到 crate 级
    file_cov: dict[str, float] = {}
    crate_hits: dict[str, list[int]] = {}
    crate_total: dict[str, list[int]] = {}

    for entry in data["data"]:
        for f in entry["files"]:
            name = f["filename"].replace("\\", "/")
            counts = f["summary"]["lines"]["count"]
            covered = f["summary"]["lines"]["covered"]
            pct = (covered / counts * 100.0) if counts else 100.0
            file_cov[name] = pct

            for crate in ("xq-core", "xq-ai", "xq-coach", "xq-protocol"):
                if f"/{crate}/" in name:
                    crate_hits.setdefault(crate, [0, 0])
                    crate_total.setdefault(crate, [0, 0])
                    crate_hits[crate][1] += covered
                    crate_total[crate][1] += counts
                    break

    failed = False

    print("=== crate 级覆盖率 ===")
    for spec in args.crate_gate:
        crate, gate = spec.split("=")
        gate = float(gate)
        acc = crate_hits.get(crate)
        if not acc or crate_total[crate][1] == 0:
            print(f"  [警告] {crate} 无覆盖率数据")
            continue
        pct = crate_hits[crate][1] / crate_total[crate][1] * 100.0
        mark = "OK " if pct >= gate else "FAIL"
        print(f"  [{mark}] {crate}: {pct:.2f}% (门槛 {gate:.0f}%)")
        if pct < gate:
            failed = True

    print("=== 逐文件覆盖率（规则模块，NFR-M.02 ≥95%）===")
    for path, gate in parse_kv(args.file_gate.split(",")).items():
        # 归一化：报告中的路径为绝对路径
        matches = [k for k in file_cov if k.endswith(path)]
        if not matches:
            print(f"  [警告] 未在报告中找到 {path}")
            continue
        pct = file_cov[matches[0]]
        mark = "OK " if pct >= gate else "FAIL"
        print(f"  [{mark}] {path}: {pct:.2f}% (门槛 {gate:.0f}%)")
        if pct < gate:
            failed = True

    if failed:
        print("\n覆盖率门禁未通过（依据 docs/01 NFR-M.02）")
        return 1
    print("\n覆盖率门禁通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
```

**覆盖率上传**：以 `codecov` 或 GitHub 的 HTML 报告形式产出，PR 中自动评论差异。**规则模块覆盖率下降 > 1 个百分点即视为门禁失败**（即使绝对值仍高于 95%），防止"高覆盖但持续下滑"。

### 5.7 阶段 11：性能基准回归检测

基准目标值来自 [docs/03 §12](03-规则引擎与领域模型.md) 与 [docs/04 §8.3](04-AI引擎设计.md)。**判定阈值也取自 docs/04 §8.3：「劣化 > 20% 告警」。**

**机制**：`main` 分支每次 push 保存基准；PR 中运行同一套 bench，与之对比。

```yaml
bench-guard:
  name: 阶段11 性能基准回归
  if: github.event_name == 'pull_request' || github.event_name == 'push'
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
      with:
        fetch-depth: 0
    - uses: dtolnay/rust-toolchain@stable
      with:
        toolchain: 1.98.1
    - uses: Swatinem/rust-cache@v2

    - name: 安装对比工具
      run: cargo install critcmp --locked

    - name: 恢复 main 基线
      uses: actions/download-artifact@v4
      with:
        name: criterion-baseline
        path: target/criterion
      continue-on-error: true   # 首次无基线时跳过

    - name: 安装 gnuplot（criterion html 报告需要）
      run: sudo apt-get update && sudo apt-get install -y gnuplot

    - name: 运行基准（保存为 candidate）
      run: cargo bench --workspace -- --save-baseline candidate

    - name: 与 main 基线对比并判定
      run: bash scripts/ci/bench_gate.sh

    - name: 上传基线（仅 main）
      if: github.ref == 'refs/heads/main'
      uses: actions/upload-artifact@v4
      with:
        name: criterion-baseline
        path: target/criterion
        retention-days: 90
```

`scripts/ci/bench_gate.sh`：

```bash
#!/usr/bin/env bash
# ============================================================
# 性能基准回归门禁
# 依据：docs/03 §12（xq-core）、docs/04 §8.3（xq-ai，劣化 > 20% 告警）
# 输出：criterion 的 change 百分比，超过阈值即失败
# ============================================================
set -euo pipefail

# 阈值（百分比）。docs/04 §8.3 明确"劣化 > 20% 告警"。
# 这里把"告警"升级为"阻塞"用于 xq-core 的 perft（正确性高度相关的基准），
# 其余基准先告警，待基线稳定后再提升为阻塞。
THRESHOLD_BLOCK_CORE=15
THRESHOLD_BLOCK_AI=20
THRESHOLD_WARN=10

REPORT="bench-cmp.txt"

if ! ls target/criterion/*/  >/dev/null 2>&1; then
  echo "[跳过] 未找到 criterion 基线数据，本次不判定"
  exit 0
fi

# critcmp 输出形如：
#   group            baseline      candidate     change
#   perft/4          1.8234 s      1.9012 s      +4.27%
critcmp baseline candidate > "${REPORT}" || true
cat "${REPORT}"

over_threshold=0

while read -r line; do
  # 提取形如 +12.34% / -5.67% 的百分比
  pct="$(printf '%s\n' "${line}" | grep -oE '[+-][0-9]+(\.[0-9]+)?%' | head -n1 || true)"
  [ -n "${pct}" ] || continue

  num="${pct%\%}"
  # 只关心劣化（正数）
  worse="$(python3 -c "print(1 if float('${num}') > 0 else 0)")"
  [ "${worse}" = "1" ] || continue

  name="$(printf '%s' "${line}" | awk '{print $1}')"

  case "${name}" in
    core/*|perft/*)
      limit="${THRESHOLD_BLOCK_CORE}" ;;
    ai/*|search/*|eval/*)
      limit="${THRESHOLD_BLOCK_AI}" ;;
    *)
      limit="${THRESHOLD_WARN}" ;;
  esac

  is_over="$(python3 -c "print(1 if float('${num}') > float('${limit}') else 0)")"
  if [ "${is_over}" = "1" ]; then
    echo "[失败] ${name} 劣化 ${pct}，超过阈值 ${limit}%"
    over_threshold=1
  elif [ "$(python3 -c "print(1 if float('${num}') > float('${THRESHOLD_WARN}') else 0)")" = "1" ]; then
    echo "[告警] ${name} 劣化 ${pct}，接近阈值"
  fi
done < "${REPORT}"

if [ "${over_threshold}" -eq 1 ]; then
  echo ""
  echo "性能基准回归门禁未通过。"
  echo "若劣化是预期的（例如为正确性付出的代价），请在 PR 描述中说明，"
  echo "并同步更新 docs/03 §12 / docs/04 §8.3 中的目标值与基线记录。"
  exit 1
fi

echo "性能基准回归门禁通过"
```

**配套的基线记录**：在仓库内维护 `docs/bench-baseline.md`（**不是**本文件），记录每个受监控基准的**实测值 + 机器规格 + 日期**。原因：criterion 的对比是相对的，机器更换会导致基线失效。该文件由 `main` 分支的 CI job 自动追加一行。

> ⚠️ **诚实声明**：本文所有"目标值"来自 [docs/03 §12](03-规则引擎与领域模型.md) 与 [docs/04 §8.3](04-AI引擎设计.md) 的**设计目标**，尚未实测。**首次实测完成后须回填**，并以实测值为基线。在基线确立前，`bench-guard` 只做相对比较（同一机器的前后差异），不做绝对阈值判定。

### 5.8 阶段 10：三端交叉编译检查

**关键区分**：`xq-core` / `xq-ai` / `xq-coach` / `xq-protocol` 是纯计算 crate，可以**在任意主机上交叉编译到三端**；而 `xq-client`（Tauri）与 `xq-server` 的检查必须在**对应原生 runner** 上做（Tauri 需要平台 WebView 头文件）。

```yaml
cross-check:
  name: 阶段10 三端检查 (${{ matrix.os }})
  strategy:
    fail-fast: false
    matrix:
      include:
        - os: ubuntu-latest
          target: x86_64-unknown-linux-gnu
        - os: windows-latest
          target: x86_64-pc-windows-msvc
        - os: macos-latest
          target: aarch64-apple-darwin
  runs-on: ${{ matrix.os }}
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
      with:
        toolchain: 1.98.1
        targets: ${{ matrix.target }}
    - uses: Swatinem/rust-cache@v2

    # ---- 1) 领域层与协议层：真正的交叉编译（在 Linux 上验证三端可行）----
    - name: 领域层三端编译检查
      if: matrix.os == 'ubuntu-latest'
      run: |
        for t in x86_64-pc-windows-msvc x86_64-unknown-linux-gnu \
                 x86_64-apple-darwin aarch64-apple-darwin; do
          echo "==> cross check $t"
          rustup target add "$t" || true
          cargo check -p xq-core -p xq-ai -p xq-coach -p xq-protocol \
            --target "$t" --locked
        done

    # ---- 2) 全 workspace：原生平台检查（含 xq-server / xq-client）----
    - name: Linux 系统依赖
      if: matrix.os == 'ubuntu-latest'
      run: |
        sudo apt-get update
        sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev \
          libayatana-appindicator3-dev librsvg2-dev libssl-dev

    - name: 全 workspace 检查
      run: cargo check --workspace --all-targets --locked
```

> **macOS 通用二进制**：发布构建需要 `x86_64-apple-darwin` + `aarch64-apple-darwin` 两个 target 并 `lipo` 合并（§6.3）。日常 CI 只跑 `aarch64` 以节省时间，`:apple:` 双架构留给发布流水线。
>
> ⚠️ **待实测项**：`x86_64-pc-windows-msvc` 从 Linux 交叉编译需要 `xwin` 或 `cargo-xwin` 提供链接器与 SDK，首次配置成本较高。**建议实施顺序**：先用 `cargo check --target x86_64-pc-windows-gnu` 做快速检查，若遇到 `windows-rs` / Tauri 相关失败，则退化为"仅在 `windows-latest` 上原生检查"。两种方案的取舍在 M0 期间由实测决定，并在实施后回写本节。

### 5.9 阶段 12：重型回归（nightly）

日常 PR 只跑单元 + 集成测试；perft 全深度、对拍题库全量、自对弈 1000 局放在 nightly，避免拖慢开发循环（细节见 [docs/12 §3、§4](12-测试策略与质量保障.md)）。

```yaml
heavy-regression:
  name: 阶段12 重型回归（nightly）
  if: github.event_name == 'schedule' || contains(github.event.pull_request.labels.*.name, 'run-heavy')
  runs-on: ubuntu-latest
  timeout-minutes: 60
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
      with:
        toolchain: 1.98.1
    - uses: Swatinem/rust-cache@v2
    - name: perft 全深度（release 构建，见 docs/03 §11.2）
      run: cargo test --release -p xq-core --test perft -- --nocapture
    - name: 对拍题库全量（≥3000 局面，见 docs/12 §3）
      run: cargo test --release -p xq-core --test golden_positions -- --nocapture
    - name: 自对弈压力（1000 局，见 docs/12 §4）
      run: cargo test --release -p xq-ai --test selfplay -- --nocapture
    - name: 素材构建期校验（见 docs/05 §9.4）
      run: cargo run --release -p xq-coach --bin validate_assets
```

---

## 6. 构建与发布

### 6.1 版本号策略（SemVer）

| 项 | 规则 |
|---|---|
| 版本载体 | `[workspace.package] version`（当前 `0.1.0`），**全 workspace 统一版本**（不做 crate 独立版本） |
| 格式 | `MAJOR.MINOR.PATCH` |
| `0.x` 阶段 | `MINOR` 递增表示**里程碑推进**（M0→M1 即 `0.1.0→0.2.0`）；`PATCH` 表示缺陷修复 |
| `1.0.0` 之后 | 标准 SemVer：破坏性变更 → `MAJOR`；新功能 → `MINOR`；修复 → `PATCH` |
| 协议版本 | `xq-protocol::PROTOCOL_VERSION` **独立于**软件版本演进（见 [docs/06 §13](06-联网对战与实时通信协议.md)） |

**为什么 workspace 统一版本**：六个 crate 是**同步发布**的单一产品，不存在"用户单独依赖 xq-core 0.3 + xq-ai 0.5"的场景。统一版本消除了"跨 crate 版本兼容矩阵"这个纯负担。

**里程碑 → 版本映射（建议，待产品确认）**：

| 里程碑 | 版本 | 依据 |
|---|---|---|
| M0 | `0.1.0` | [README §8](../README.md) / [docs/13 §M0](13-路线图与里程碑.md)：规则内核可信 |
| M1 | `0.2.0` | [docs/01 §8.1](01-需求规格说明书.md)：离线版准入 |
| M2 | `0.3.0` | [docs/01 §8.2](01-需求规格说明书.md)：联网版准入 |
| M3 | `0.4.0` | [docs/01 §8.3](01-需求规格说明书.md)：完整版准入 |
| M4 | `0.5.0` / `1.0.0` | M4 无 docs/01 §8 对应条款；**是否发布 `1.0.0` 需产品确认** |

### 6.2 tag 规范

| 项 | 规则 |
|---|---|
| 格式 | `v` + 版本号，如 `v0.2.0` |
| 预发布 | `v0.2.0-rc.1`、`v0.2.0-beta.2` |
| tag 附注 | **必须**用 annotated tag，附注内容 = CHANGELOG 条目 |
| tag 触发 | 只有 tag 匹配 `v[0-9]+.[0-9]+.[0-9]+*` 才触发发布流水线 |
| tag 保护 | tag 一旦推送**不得删除或移动**（`git push --force` tag 属禁令）。发现错误则发新 patch 版本 |

```bash
# 打 tag 的标准流程
git switch main && git pull --ff-only
cargo set-version --workspace 0.2.0          # 或手工改 Cargo.toml
git commit -am "chore(release): 发布 v0.2.0"
git tag -a v0.2.0 -m "v0.2.0 —— M1 离线可下（docs/13 §M1）"
git push origin main --follow-tags
```

### 6.3 制品命名与打包矩阵

**命名规范**：`yidao-<version>-<os>-<arch>.<ext>`

| 平台 | Target | 制品 | 命名示例 |
|---|---|---|---|
| Windows | `x86_64-pc-windows-msvc` | MSI | `yidao-0.2.0-windows-x64.msi` |
| Windows | `x86_64-pc-windows-msvc` | NSIS 安装包 | `yidao-0.2.0-windows-x64-setup.exe` |
| Linux | `x86_64-unknown-linux-gnu` | AppImage | `yidao-0.2.0-linux-x86_64.AppImage` |
| Linux | `x86_64-unknown-linux-gnu` | deb | `yidao-0.2.0-linux-amd64.deb` |
| Linux | `x86_64-unknown-linux-gnu` | rpm | `yidao-0.2.0-linux-x86_64.rpm` |
| macOS | `universal-apple-darwin` | DMG（通用二进制） | `yidao-0.2.0-macos-universal.dmg` |

**平台要求**（来自 [NFR-C](01-需求规格说明书.md) 与 [docs/08 §15](08-客户端设计.md)）：

| 项 | 要求 |
|---|---|
| Windows | Windows 10 及以上（x64）；WebView2 运行时（Win10+ 一般内置，否则引导安装） |
| Linux | glibc 2.31+（Ubuntu 20.04+ / Fedora / Arch） |
| macOS | macOS 12+，Intel + Apple Silicon 通用二进制 |
| 体积 | ≤ 30 MB（NFR-P.09，**设计目标，待实测**） |

**Linux 系统依赖**（`deb` / `rpm` 需在包元数据中声明）：

```bash
# Debian / Ubuntu —— 开发与打包机
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev \
  libayatana-appindicator3-dev librsvg2-dev \
  build-essential curl wget file libssl-dev

# Fedora
sudo dnf install webkit2gtk4.1-devel gtk3-devel \
  libappindicator-gtk3-devel librsvg2-devel openssl-devel

# Arch 系（含 CachyOS）
sudo pacman -S webkit2gtk-4.1 gtk3 libappindicator-gtk3 librsvg openssl
```

> `AppImage` 把依赖打包在内，终端用户无需安装；`deb`/`rpm` 在包元数据中声明依赖由包管理器解析（[ADR-002](14-决策记录ADR.md#adr-002) 权衡表已记录此代价）。

### 6.4 发布流水线

```yaml
name: release

on:
  push:
    tags: ['v[0-9]+.[0-9]+.[0-9]+*']
  workflow_dispatch:

permissions:
  contents: write

jobs:
  # ---------- 发布前置门禁：不允许把未通过测试的版本发出去 ----------
  preflight:
    name: 发布前置门禁
    uses: ./.github/workflows/ci.yml
    secrets: inherit

  # ---------- 三端打包 ----------
  build-tauri:
    name: 打包 ${{ matrix.platform }}
    needs: [preflight]
    strategy:
      fail-fast: false
      matrix:
        include:
          - platform: macos-latest
            args: --target universal-apple-darwin
            rust_targets: aarch64-apple-darwin,x86_64-apple-darwin
          - platform: ubuntu-latest
            args: ''
            rust_targets: ''
          - platform: windows-latest
            args: ''
            rust_targets: ''
    runs-on: ${{ matrix.platform }}
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.98.1
          targets: ${{ matrix.rust_targets }}

      - name: Linux 系统依赖
        if: matrix.platform == 'ubuntu-latest'
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev \
            libayatana-appindicator3-dev librsvg2-dev patchelf

      - uses: pnpm/action-setup@v4
        with:
          version: 10
      - uses: actions/setup-node@v4
        with:
          node-version: 22
          cache: pnpm
          cache-dependency-path: frontend/pnpm-lock.yaml

      - uses: Swatinem/rust-cache@v2
        with:
          key: release-${{ matrix.platform }}

      - name: 安装前端依赖
        working-directory: frontend
        run: pnpm install --frozen-lockfile

      - name: 构建并发布
        uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          # ---- macOS 签名与公证（仅 macOS job 需要）----
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
          APPLE_CERTIFICATE_PASSWORD: ${{ secrets.APPLE_CERTIFICATE_PASSWORD }}
          APPLE_SIGNING_IDENTITY: ${{ secrets.APPLE_SIGNING_IDENTITY }}
          APPLE_ID: ${{ secrets.APPLE_ID }}
          APPLE_PASSWORD: ${{ secrets.APPLE_PASSWORD }}
          APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
          # ---- Tauri updater 签名 ----
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        with:
          tagName: ${{ github.ref_name }}
          releaseName: '弈道 ${{ github.ref_name }}'
          releaseBody: '见 CHANGELOG.md'
          releaseDraft: true
          prerelease: false
          args: ${{ matrix.args }}

      - name: 体积校验（NFR-P.09 ≤ 30 MB）
        shell: bash
        run: bash scripts/ci/check_artifact_size.sh 30

  # ---------- 制品归档 ----------
  archive:
    name: 归档制品并计算校验和
    needs: [build-tauri]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v4
      - name: 生成 SHA256SUMS
        run: find . -type f \( -name '*.msi' -o -name '*.exe' -o -name '*.AppImage' \
             -o -name '*.deb' -o -name '*.rpm' -o -name '*.dmg' \) \
             -exec sha256sum {} + | sort -k2 > SHA256SUMS.txt
      - uses: actions/upload-artifact@v4
        with:
          name: release-artifacts
          path: SHA256SUMS.txt
```

**服务端发布**（独立于客户端版本，可与 tag 绑定或单独触发）：

```yaml
  build-server:
    name: 构建服务端容器镜像
    needs: [preflight]
    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: write
    steps:
      - uses: actions/checkout@v4
      - uses: docker/setup-buildx-action@v3
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      - uses: docker/build-push-action@v6
        with:
          context: .
          file: deploy/Dockerfile.server
          push: true
          tags: |
            ghcr.io/${{ github.repository }}/xq-server:${{ github.ref_name }}
            ghcr.io/${{ github.repository }}/xq-server:latest
          cache-from: type=gha
          cache-to: type=gha,mode=max
```

### 6.5 macOS 签名与公证要点

> 未签名/未公证的 macOS 应用在用户机器上会提示「已损坏，无法打开」——这不是可选的优化，而是**能否分发的门槛**。

| 步骤 | 要点 |
|---|---|
| **1. 证书** | 使用 Apple Developer Program 的 **Developer ID Application** 证书（不是 Mac App Store 的 Apple Distribution）。导出为 `.p12`，Base64 编码后存为 `APPLE_CERTIFICATE` secret |
| **2. 证书密码** | `APPLE_CERTIFICATE_PASSWORD` |
| **3. 签名身份** | `APPLE_SIGNING_IDENTITY`，形如 `Developer ID Application: XXX (TEAMID)` |
| **4. 公证凭据** | 优先用 **App Store Connect API Key**（`APPLE_API_ISSUER` / `APPLE_API_KEY` / byte-safe 的 `APPLE_API_KEY_PATH`），比 `APPLE_ID` + 应用专用密码更稳定、不受 2FA 影响。**两种方式都保留，API Key 优先** |
| **5. Team ID** | `APPLE_TEAM_ID`，10 位，公证与签名都需要 |
| **6. 通用二进制** | `--target universal-apple-darwin`。Tauri 会在构建脚本中自动 `lipo` 合并。**注意**：若新增了平台相关的 native 依赖，通用二进制可能是唯一会失败的目标——必须有一个 macOS CI job 常驻 |
| **7. entitlements** | 默认 Tauri entitlements 一般够用。若后续开启沙盒或使用特定 API（如网络客户端），需显式声明 `com.apple.security.network.client` |
| **8. 公证耗时** | `notarytool` 通常数分钟，**偶尔会等待数十分**。发布 job 的超时需 ≥ 60 分钟，且要能容忍重试 |
| **9. 验证** | 发布后必须实测：`spctl -a -vvv -t install yidao-*.app` 与 `xcrun stapler validate` |

**失败排查顺序**（写在这里是为了避免每次重复踩坑）：

1. 证书是否过期 / 是否为 Developer ID 类型；
2. `APPLE_TEAM_ID` 与证书中的 Team 是否一致；
3. 是否遗漏了嵌套二进制（Tauri 的 sidecar）的签名（必须从内到外签名）；
4. entitlements 是否声明了必需的权限；
5. 公证日志：`xcrun notarytool log <submission-id> --key ...`。

### 6.6 CHANGELOG

- 由 `git-cliff` 依据 Conventional Commits 自动生成，配置 `cliff.toml`；
- 生成时机：打 tag 时由发布流水线生成并追加；
- **不手工维护**：手工维护的 CHANGELOG 在第三个版本就会与 git 历史脱节。

```toml
# cliff.toml（节选）
[changelog]
header = "# 变更日志\n\n本项目遵循 [Semantic Versioning](https://semver.org/)。\n"
body = """
{% if version %}\
## [{{ version | trim_start_matches(pat="v") }}] - {{ timestamp | date(format="%Y-%m-%d") }}
{% else %}\
## [未发布]
{% endif %}\
{% for group, commits in commits | group_by(attribute="group") %}
### {{ group | striptags | trim }}
{% for commit in commits %}
- {{ commit.message | upper_first }}{% if commit.scope %} (`{{ commit.scope }}`){% endif %}
{%- endfor %}
{% endfor %}\n
"""
[git]
conventional_commits = true
filter_unconventional = true
split_commits = false
commit_parsers = [
  { message = "^feat",     group = "新功能" },
  { message = "^fix",      group = "缺陷修复" },
  { message = "^perf",     group = "性能" },
  { message = "^refactor", skip  = true },
  { message = "^docs",     skip  = true },
  { message = "^test",     skip  = true },
  { message = "^ci",       skip  = true },
  { message = "^chore",    skip  = true },
  { message = "^style",    skip  = true },
]
```

---

## 7. 本地开发环境

### 7.1 必需工具与版本

| 工具 | 版本 | 安装 | 校验命令 |
|---|---|---|---|
| Rust | **1.98.1**（`rust-toolchain.toml` 锁定） | `rustup` | `rustc --version` |
| rustfmt / clippy | 随 toolchain | `rustup component add rustfmt clippy` | `cargo clippy -V` |
| Node.js | 22.x LTS 或 24.x | 版本管理器 | `node -v` |
| pnpm | 10.x | `corepack enable && corepack prepare pnpm@latest --activate` | `pnpm -v` |
| Docker + Compose | 最新稳定版 | 官方安装包 | `docker compose version` |
| PostgreSQL | 16+（**用 Docker 起，不装本机**） | `docker compose up -d postgres` | `psql --version`（容器内） |
| Redis | 7.x（同上） | `docker compose up -d redis` | `redis-cli ping` |
| `cargo-nextest` | 最新 | `cargo install cargo-nextest --locked` | `cargo nextest --version` |
| `cargo-llvm-cov` | 最新 | `cargo install cargo-llvm-cov --locked` | `cargo llvm-cov --version` |
| `cargo-public-api` | 最新 | `cargo install cargo-public-api --locked` | `cargo public-api -V` |
| `critcmp` | 最新 | `cargo install critcmp --locked` | `critcmp -V` |
| `git-cliff` | 最新 | `cargo install git-cliff --locked` | `git-cliff -V` |

**工具链固定**：`rust-toolchain.toml`（内容见 [docs/02 §11](02-系统架构设计.md)）确保所有人与 CI 使用同一版本。**禁止**用手工 `rustup default` 覆盖。

**Linux 额外系统依赖**：见 §6.3。开发客户端时（`pnpm tauri dev`）必须先装 `libwebkit2gtk-4.1-dev` 等。

### 7.2 Docker Compose（PostgreSQL + Redis）

`docker-compose.yml`：

```yaml
# ============================================================
# 弈道 · 本地开发基础设施
# 用法：
#   docker compose up -d            启动 PostgreSQL + Redis
#   docker compose logs -f postgres 查看日志
#   docker compose down             停止（保留数据卷）
#   docker compose down -v          停止并删除数据（清库）
# ============================================================

name: yidao-dev

services:
  postgres:
    image: postgres:16-alpine
    container_name: yidao-postgres
    restart: unless-stopped
    environment:
      POSTGRES_USER: yidao
      POSTGRES_PASSWORD: yidao_dev_pw      # 仅本地开发；生产密码只从环境变量注入
      POSTGRES_DB: yidao
      # 中文排序与大小写不敏感检索（昵称、聊天）
      POSTGRES_INITDB_ARGS: "--encoding=UTF8 --locale=C"
      TZ: Asia/Shanghai
    ports:
      - "5432:5432"
    volumes:
      - pgdata:/var/lib/postgresql/data
      - ./scripts/dev/init-test-db.sql:/docker-entrypoint-initdb.d/10-init-test-db.sql:ro
    command:
      - postgres
      # 开发期日志更全，便于看慢查询
      - -c
      - log_min_duration_statement=200
      - -c
      - log_statement=none
      - -c
      - max_connections=200
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U yidao -d yidao"]
      interval: 5s
      timeout: 3s
      retries: 20
      start_period: 10s

  redis:
    image: redis:7-alpine
    container_name: yidao-redis
    restart: unless-stopped
    ports:
      - "6379:6379"
    volumes:
      - redisdata:/data
    # AOF 开启：本地也能演练"重启后房间恢复"（NFR-R.03）
    command:
      - redis-server
      - --appendonly
      - "yes"
      - --appendfsync
      - everysec
      - --maxmemory
      - 512mb
      - --maxmemory-policy
      - noeviction        # 房间状态不可被淘汰，与 ADR-010 一致
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 5s
      timeout: 3s
      retries: 20
      start_period: 5s

  # 可选：数据库管理界面（默认不启动，用 --profile tools 启用）
  adminer:
    image: adminer:latest
    container_name: yidao-adminer
    profiles: ["tools"]
    ports:
      - "8081:8080"
    environment:
      ADMINER_DEFAULT_SERVER: postgres
    depends_on:
      postgres:
        condition: service_healthy

volumes:
  pgdata:
    name: yidao-pgdata
  redisdata:
    name: yidao-redisdata
```

`scripts/dev/init-test-db.sql`（首次初始化时创建测试库，见 [docs/12 §11](12-测试策略与质量保障.md)）：

```sql
-- 独立测试库：集成测试使用，与开发库隔离
CREATE DATABASE yidao_test OWNER yidao;

-- 集成测试需要建表权限（测试用例自行跑迁移）
GRANT ALL PRIVILEGES ON DATABASE yidao_test TO yidao;

-- 测试库允许更激进的清理操作
ALTER DATABASE yidao_test SET synchronous_commit = off;
```

### 7.3 `.env.example`（**必须提交仓库**）

```dotenv
# ============================================================
# 弈道 · 环境变量样例文件
# 用法：
#   cp .env.example .env      然后按需修改
# .env 已被 .gitignore 忽略，**绝不入库**（见 §8.3）
# ============================================================

# ---- 服务端 ----
XQ__SERVER__HOST=0.0.0.0
XQ__SERVER__PORT=8080
XQ__SERVER__MAX_CONNECTIONS=10000

# ---- 数据库 ----
XQ__DATABASE__URL=postgres://yidao:yidao_dev_pw@127.0.0.1:5432/yidao
XQ__DATABASE__MAX_CONNECTIONS=20

# ---- Redis ----
XQ__REDIS__URL=redis://127.0.0.1:6379

# ---- 对局参数（与 config/default.toml 对应，见 docs/02 §8）----
XQ__GAME__ROOM_IDLE_TIMEOUT_SECS=900
XQ__GAME__DISCONNECT_FORFEIT_SECS=120
XQ__GAME__SPECTATOR_DELAY_MS=3000
XQ__GAME__SPECTATOR_LIMIT=50
XQ__GAME__MAX_UNDO_PER_GAME=3

# ---- 讲解 ----
XQ__COACH__LLM_ENABLED=false
XQ__COACH__LLM_TIMEOUT_MS=3000
XQ__COACH__LLM_DAILY_BUDGET=100000

# ---- 日志 ----
XQ__LOG__LEVEL=debug
XQ__LOG__FORMAT=pretty

# ============================================================
# 以下为**敏感项**。样例文件只给占位符，真实值只存在本地 .env
# 或部署平台的 Secret 管理中。**任何真实密钥都不得提交到仓库。**
# ============================================================

# JWT 签名密钥（≥32 字节高熵随机值；生成：openssl rand -base64 48）
XQ__AUTH__JWT_SECRET=CHANGE_ME__generate_with_openssl_rand_base64_48

# LLM 服务凭据（客户端**禁止**持有，见 NFR-S.09）
XQ__COACH__LLM_API_KEY=CHANGE_ME
XQ__COACH__LLM_BASE_URL=https://example.invalid/v1
```

### 7.4 常用命令速查表

| 目的 | 命令 |
|---|---|
| **基础设施** | |
| 启动 PG + Redis | `docker compose up -d` |
| 查看健康状态 | `docker compose ps` |
| 清库重来 | `docker compose down -v && docker compose up -d` |
| 连 PG | `docker compose exec postgres psql -U yidao -d yidao` |
| 连 Redis | `docker compose exec redis redis-cli` |
| **Rust 开发** | |
| 编译全 workspace | `cargo build --workspace --all-targets` |
| 只编领域层（快） | `cargo build -p xq-core -p xq-ai -p xq-coach` |
| 单 crate 测试 | `cargo nextest run -p xq-core` |
| 全量测试 | `cargo nextest run --workspace` |
| 文档测试 | `cargo test --doc --workspace` |
| 集成测试（需 PG/Redis） | `cargo nextest run --workspace --test '*'` |
| perft 校验 | `cargo test --release -p xq-core --test perft -- --nocapture` |
| 对拍题库 | `cargo test --release -p xq-core --test golden_positions` |
| 自对弈压力 | `cargo test --release -p xq-ai --test selfplay -- --nocapture` |
| **质量** | |
| 格式化 | `cargo fmt --all` |
| 格式检查 | `cargo fmt --all -- --check` |
| Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| 依赖断言（ADR-005） | `bash scripts/ci/assert_no_io_deps.sh` |
| 覆盖率（HTML） | `cargo llvm-cov --workspace --html --open` |
| 覆盖率门禁 | `bash scripts/ci/coverage_gate.sh` |
| 公开 API 对比 | `bash scripts/ci/check_public_api.sh` |
| **性能** | |
| 全量基准 | `cargo bench --workspace` |
| 保存基线 | `cargo bench --workspace -- --save-baseline main` |
| 与基线对比 | `cargo bench --workspace -- --baseline main` |
| 基准门禁 | `bash scripts/ci/bench_gate.sh` |
| **服务端** | |
| 跑迁移 | `cargo run -p xq-server --bin migrate up` |
| 回滚一步迁移 | `cargo run -p xq-server --bin migrate down --steps 1` |
| 启动服务端 | `cargo run -p xq-server` |
| 启动（watch 模式） | `cargo watch -x 'run -p xq-server'` |
| **客户端** | |
| 安装前端依赖 | `pnpm -C frontend install` |
| 前端开发服务器（仅浏览器） | `pnpm -C frontend dev` |
| Tauri 开发模式 | `pnpm -C frontend tauri dev` |
| Tauri 打包（本地） | `pnpm -C frontend tauri build` |
| **前端质量** | |
| ESLint | `pnpm -C frontend exec eslint . --max-warnings 0` |
| Prettier 检查 | `pnpm -C frontend exec prettier --check .` |
| 类型检查 | `pnpm -C frontend exec tsc --noEmit` |
| 前端测试 | `pnpm -C frontend exec vitest run` |
| 前端测试（watch） | `pnpm -C frontend exec vitest` |

---

## 8. 配置与密钥管理

### 8.1 分层配置（`config` crate）

**分层顺序（后者覆盖前者）**，与 [docs/02 §8](02-系统架构设计.md) 一致：

```
① 代码内默认值（Config::default()）
    ↓ 被覆盖
② config/default.toml        —— 入库，全员共享的基准配置
    ↓ 被覆盖
③ config/local.toml          —— 不入库（.gitignore），个人覆盖
    ↓ 被覆盖
④ 环境变量（XQ__ 前缀）        —— 部署环境注入；**敏感项的唯一来源**
    ↓ 被覆盖
⑤ 命令行参数                  —— 临时调试，优先级最高
```

**Rust 侧加载**：

```rust
// crates/xq-server/src/config.rs
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub game: GameConfig,
    pub ai: AiConfig,
    pub coach: CoachConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub auth: AuthConfig,
    pub log: LogConfig,
}

pub fn load() -> anyhow::Result<AppConfig> {
    let mut builder = config::Config::builder()
        .add_source(config::File::with_name("config/default"))
        // local.toml 存在则覆盖，不存在不报错
        .add_source(config::File::with_name("config/local").required(false))
        // 环境变量：XQ__SERVER__PORT → server.port
        .add_source(
            config::Environment::with_prefix("XQ")
                .prefix_separator("__")
                .separator("__"),
        );

    // 敏感项强制只来自环境变量：即使 default.toml 里写了占位符也不生效
    builder = builder
        .set_override_option(
            "auth.jwt_secret",
            std::env::var("XQ__AUTH__JWT_SECRET").ok(),
        )?;

    let cfg: AppConfig = builder.build()?.try_deserialize()?;
    cfg.validate()?;
    Ok(cfg)
}
```

**配置项的三条硬规则**：

| 规则 | 说明 |
|---|---|
| **默认值必须安全** | 默认关闭 LLM（`coach.llm_enabled = false`）、默认不暴露调试端点、默认开启鉴权 |
| **启动时校验** | `validate()` 在 `main` 最开始调用。缺少必需的敏感项 → **启动失败并打出明确错误**，绝不"用默认空值静默运行" |
| **新增配置必须三处同步** | `config/default.toml` + `.env.example` + 本节或 [docs/02 §8](02-系统架构设计.md)。CI 应校验 `default.toml` 的 key 集合是 `.env.example` 的子集 |

### 8.2 环境变量命名规范

```
XQ__<SECTION>__<FIELD>            （双下划线分级）
 ^^   ^^^^^^^^^   ^^^^^^^
 前缀   配置节      字段（SCREAMING_SNAKE_CASE）
```

| 规则 | 例 |
|---|---|
| 统一 `XQ__` 前缀 | `XQ__SERVER__PORT` |
| 层级用 `__`（双下划线），单词内用 `_`（单下划线） | `XQ__GAME__DISCONNECT_FORFEIT_SECS` |
| 布尔用 `true` / `false` | `XQ__COACH__LLM_ENABLED` |
| 数值带单位后缀 | `_SECS` / `_MS` / `_BYTES` / `_MB` |
| 数组用 JSON 或逗号分隔（二选一，**全项目统一用逗号分隔**） | `XQ__LOG__TARGETS=axum,sqlx` |
| 敏感项集中在 `AUTH` / `COACH` 等节下 | `XQ__AUTH__JWT_SECRET` |

**禁止**：`XQ_TOKEN`、`XQ_SECRET_KEY` 这类无分级、无归属的扁平命名。

### 8.3 密钥管理原则（**绝不入库**）

| 原则 | 落地手段 |
|---|---|
| **K-1 密钥绝不入库** | `.gitignore` 显式忽略 `.env`、`config/local.toml`、`*.p12`、`*.pem`、`*.key`、`*.jks` |
| **K-2 提交前扫描** | CI + pre-commit 双层 `gitleaks` 扫描（见下） |
| **K-3 样例文件只放占位符** | `.env.example` 中所有敏感项值为 `CHANGE_ME...`，且明确指出生成方式 |
| **K-4 生产密钥只在 Secret 管理** | GitHub Actions Secrets / 部署平台 Secret（如 K8s Secret、云 Secret Manager）。**不写入任何配置文件** |
| **K-5 客户端不含任何服务端密钥** | NFR-S.09。LLM 增强必须经自家服务端代理（[docs/05 §6.1](05-战法讲解引擎设计.md)） |
| **K-6 密钥轮换有路径** | JWT 密钥支持双密钥并行验证（`kid`），轮换时新旧并存一个 token 周期 |
| **K-7 泄露有响应流程** | 一旦发现密钥入库：① **立即轮换**（不是先删 commit）；② 用 `git filter-repo` 清理历史；③ 评估影响范围并记录在 ADR |

`.gitignore` 关键片段：

```gitignore
# ---- 密钥与本地配置（绝不入库）----
.env
.env.*
!.env.example
config/local.toml
*.p12
*.pem
*.key
*.jks
*.keystore
secrets/
.tauri-keys/

# ---- 构建产物 ----
target/
frontend/dist/
frontend/node_modules/
frontend/src-tauri/target/

# ---- 测试与覆盖率产物 ----
target/coverage/
*.profraw
frontend/coverage/
criterion-baseline/

# ---- 工具与编辑器 ----
.DS_Store
Thumbs.db
.idea/
.vscode/*
!.vscode/settings.json
```

**`gitleaks` 配置**（`.gitleaks.toml`）：

```toml
title = "弈道 · 密钥扫描规则"

[extend]
useDefault = true   # 继承内置规则集（AWS / GitHub / 私钥 / 通用高熵串）

[[rules]]
id = "yidao-jwt-secret"
description = "硬编码的 JWT 密钥"
regex = '''(?i)jwt[_-]?secret\s*[:=]\s*["']([A-Za-z0-9+/=]{24,})["']'''
tags = ["key", "jwt"]

[[rules]]
id = "yidao-llm-api-key"
description = "硬编码的 LLM API Key"
regex = '''(?i)(sk|api)[_-]?key\s*[:=]\s*["']([A-Za-z0-9\-_]{20,})["']'''
tags = ["key", "llm"]

[allowlist]
description = "允许的样例值"
paths = [
  '''\.env\.example$''',
  '''docs/''',
  '''scripts/ci/''',
]
regexes = [
  '''CHANGE_ME''',
  '''example\.invalid''',
  '''yidao_dev_pw''',
]
```

**CI 中的用法**：

```yaml
secret-scan:
  name: 密钥扫描
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
      with:
        fetch-depth: 0        # gitleaks 需要完整历史
    - uses: gitleaks/gitleaks-action@v2
      env:
        GITLEAKS_CONFIG: .gitleaks.toml
        GITLEAKS_ENABLE_COMMENTS: false
```

**pre-commit 钩子**（`.githooks/pre-commit`，配合 `git config core.hooksPath .githooks`）：

```bash
#!/usr/bin/env sh
set -e

if command -v gitleaks >/dev/null 2>&1; then
  gitleaks protect --staged --config .gitleaks.toml --redact
else
  echo "[跳过] 未安装 gitleaks，建议：brew install gitleaks / scoop install gitleaks"
fi
```

---

## 9. 日志与可观测规范

### 9.1 日志分级使用准则

`tracing` + `tracing-subscriber`，JSON 输出（[docs/02 §9](02-系统架构设计.md)）。

| 级别 | 使用场景 | 生产默认 | 示例 |
|---|---|---|---|
| `ERROR` | **需要人介入**的故障。每一条都应对应一个告警或已知的待修问题 | ✅ 输出 | 数据库连接池耗尽、迁移失败、LLM 连续熔断 |
| `WARN` | **异常但可自愈/可降级**的情况。累积起来需要观察 | ✅ 输出 | LLM 超时降级为模板、客户端着法与快照不一致、Redis 重连一次成功 |
| `INFO` | **业务里程碑**。一个对局的完整生命周期应能仅凭 INFO 复原 | ✅ 输出 | 建房、开局、终局、结算、用户登录成功 |
| `DEBUG` | **开发与排障细节**。高频，生产关闭 | ❌ 关闭（可按模块临时开） | 帧收发明细、房间状态转移、搜索深度迭代 |
| `TRACE` | **逐帧/逐节点的极细粒度**。只在定位特定问题时临时开 | ❌ 关闭 | 每个搜索节点的 alpha/beta 窗口 |

**分级判断的三个反常识点**：

1. **"能自愈的不算 ERROR"**：Redis 断连后自动重连成功 → `WARN`，不是 `ERROR`。把可恢复事件记成 ERROR 会训练运维忽略告警。
2. **"用户操作失败不一定是 ERROR"**：非法着法被拒是**正常业务流程**（NFR-S.01 的预期行为）→ `INFO` 或 `DEBUG`，不是 ERROR。但**同一账号的高频非法着法**→ `WARN`（反作弊信号，见 [docs/07 §7](07-匹配排行榜与反作弊.md)）。
3. **"什么都不记也是问题"**：`ERROR` 里如果没有 `request_id`，这条日志的价值接近于零。

**动态调整**：`RUST_LOG` 支持按模块精细控制。生产通过 `XQ__LOG__LEVEL` 设置全局级别，并允许 `XQ__LOG__DIRECTIVES` 追加针对模块的覆盖：

```bash
# 全局 info，但 sqlx 只到 warn（SQL 日志太吵），room 模块开到 debug 排查问题
XQ__LOG__LEVEL=info
XQ__LOG__DIRECTIVES=sqlx=warn,xq_server::room=debug
```

### 9.2 必带字段

**全链路上下文**必须通过 `tracing::Span` 携带，不手工在每条日志里拼字符串。字段名统一（来自 [docs/02 §9](02-系统架构设计.md) 与 [NFR-M.04](01-需求规格说明书.md)）：

| 字段 | 类型 | 何时必带 | 用途 |
|---|---|---|---|
| `request_id` | string (UUID v7) | **所有** HTTP 请求与 WS 会话 | 用户报障 → 一次查询定位全链路 |
| `room_id` / `room_code` | string | 房间相关的所有日志 | 定位单个房间的问题 |
| `game_id` | string (UUID v7) | 对局相关的所有日志 | 与 `game_moves` / `game_coach_notes` 关联 |
| `player_id` / `user_id` | string (UUID v7) | 涉及具体用户的日志 | 审计与反作弊 |
| `session_id` | string | WS 连接相关 | 追踪单条连接的生命周期 |
| `seq` | u64 | 对局帧相关 | 定位乱序 / 丢帧 / 补帧问题 |
| `span` 上下文 | — | 由 `#[instrument]` 自动注入 | 避免手工传递遗漏 |

**推荐写法**：

```rust
use tracing::{info, instrument, warn};

// ✅ 用 #[instrument] 自动注入字段，函数体内不再手工拼接
#[instrument(
    skip(self, req),
    fields(
        request_id = %req.request_id,
        room_id = %req.room_id,
        player_id = %req.player_id,
    )
)]
async fn handle_move(&self, req: MoveRequest) -> Result<MoveApplied, ApiError> {
    info!(client_move_no = req.client_move_no, "收到走子请求");

    match self.validate(&req).await {
        Ok(applied) => {
            info!(seq = applied.seq, notation = %applied.notation, "走子已应用");
            Ok(applied)
        }
        Err(e) => {
            // 业务拒绝：WARN 而非 ERROR；携带拒绝原因便于统计
            warn!(reason = ?e, "走子被拒绝");
            Err(e.into())
        }
    }
}
```

**客户端侧**：Tauri 应用的日志写入本地文件（滚动保留），并提供"导出日志"入口（**导出前先脱敏**）。字段规范与服务端一致，便于两端日志对照排查。

### 9.3 禁止记录的内容（红线）

> 以下是**硬红线**。违反者按 [docs/12 §10](12-测试策略与质量保障.md) 的 **P1 缺陷**处理。

| 禁止记录 | 说明 | 替代做法 |
|---|---|---|
| **密码** | 明文、哈希、甚至"密码长度"都不要记 | 只记"鉴权失败"，不带任何凭据信息 |
| **token（access / refresh）** | JWT 原文、refresh token、`session_token` | 记 `jti` / `token_version` / `user_id`；如需关联记前 8 位哈希前缀 |
| **API Key / 密钥** | LLM API Key、JWT 签名密钥、数据库密码 | 记 `key_id` 或 `provider` 名 |
| **完整手机号 / 邮箱** | 属于个人隐私 | 脱敏：`138****5678`、`u***@example.com`（`MaskedPhone` / `MaskedEmail` 类型，只在构造时脱敏） |
| **完整身份证 / 实名信息** | 本期不采集；若未来采集则一律不落日志 | — |
| **HTTP 请求体原文** | 可能包含密码、token、聊天内容 | 只记字段名与长度，或明确的脱敏白名单字段 |
| **棋盘完整 FEN + 用户标识的组合** | 单独的 FEN 不敏感；但与用户绑定的行为数据属隐私 | 调试时用 `debug` 级别，且生产默认关闭 |
| **内部堆栈 / SQL 语句（对外）** | 对外错误消息不得含（[docs/02 §10](02-系统架构设计.md) 原则 1） | 对外返回错误码；完整信息只进服务端日志 |

**实现保障**：

```rust
/// 包装类型：只有脱敏后的形态可以被格式化到日志
pub struct MaskedPhone(String);

impl MaskedPhone {
    pub fn new(raw: &str) -> Self {
        // 前 3 后 4，中间固定 4 个星号（不管原长度，避免泄露长度信息）
        let masked = if raw.chars().count() >= 7 {
            let head: String = raw.chars().take(3).collect();
            let tail: String = raw.chars().rev().take(4).collect::<String>().chars().rev().collect();
            format!("{head}****{tail}")
        } else {
            "***".to_string()
        };
        Self(masked)
    }
}

impl std::fmt::Display for MaskedPhone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// 故意不实现 Debug 的原始形态；若必须实现，也输出脱敏值
impl std::fmt::Debug for MaskedPhone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("MaskedPhone").field(&self.0).finish()
    }
}
```

> **用类型而不用约定**：约定"记得脱敏"会在第 37 个开发者的第 200 次提交上失效。用 `MaskedPhone` 这种类型，让"记录明文"在类型层面就无法表达。

### 9.4 可观测性配套

| 维度 | 手段 | 关键指标（来自 [docs/02 §9](02-系统架构设计.md)） |
|---|---|---|
| 指标 | Prometheus 格式 `/metrics` | `ws.frame.latency`、`game.move.validate_duration`、`coach.generate.duration`、`ai.search.nodes`、`ai.search.duration` |
| 健康检查 | `GET /healthz`（存活）、`GET /readyz`（依赖就绪） | `/readyz` 必须在 DB 与 Redis 均可用时才返回 200 |
| 链路追踪 | OpenTelemetry（**M4 再评估**，M0~M3 用 `request_id` 贯穿即可） | — |
| 审计 | 关键动作写 `audit_logs` | 登录、建房、封禁、赛季重置、管理员操作 |

**`/metrics` 的暴露方式**：见 [docs/10 §12 D-15](10-API接口规范.md)——约定不用业务端口，具体方案待 M4 部署阶段确定。

---

## 10. 代码评审检查清单（Checklist）

> 本节可直接复制进 PR 描述使用。**每一项都设计为可判定**——不能判定的项不应进 Checklist。

### 10.1 领域层（`xq-core` / `xq-ai` / `xq-coach`）专用

```
□ 无 unwrap / expect / panic / 裸索引（clippy 已 deny，本地是否已 allow？每个 allow 是否有理由注释？）
□ 无 IO 依赖、无 SystemTime、无 thread_rng（ADR-005）
□ 无 anyhow（领域层必须用 thiserror 具名错误）
□ 规则的每一处非平凡判定都标注了 docs/03 §x.x 的来源
□ 无分配型 API 被用在热点路径（搜索内层不得出现 Vec::new / clone / to_string）
□ 公开 API 变更是否必要？能否改为 pub(crate)？
□ 新增公开枚举是否加了 #[non_exhaustive]？
□ 契约型函数（如 analyze 永不失败）是否在 doc 注释中重申了契约 + 测试文件名？
```

### 10.2 规则正确性（`xq-core` 专项）

```
□ 改动的规则行为有对应测试，且该测试在改动前会失败（无法自证 → 请补充反例）
□ 是否运行了 perft 全深度并与 docs/03 §11.2 的记录对比？结果附在 PR 中
□ 是否运行了对拍题库全量？通过率是否仍为 100%？
□ 新增的规则边界是否加入了对拍题库（见 docs/12 §3）？
□ 若改变了 perft 数值：是发现了原实现 bug，还是引入了 bug？必须给出逐项推导
□ 记谱 / FEN 的往返属性测试是否仍通过（≥10000 次）？
```

### 10.3 引擎（`xq-ai`）专项

```
□ 是否附了 criterion 前后对比？劣化是否在用 PR 说明中论证？
□ 搜索结果是否仍然确定（相同种子 + NodeLimitedStop → 相同结果）？
□ 镜像对称性测试是否仍通过（eval(pos) == -eval(mirror(pos))）？
□ 难度参数（容差 / 失误率 / 深度）改动是否同步了 docs/04 §6？
□ 若引入了新的剪枝/启发：是否有"关闭该优化后结果应相同"的对照测试？
```

### 10.4 讲解（`xq-coach`）专项

```
□ 模板新增/修改是否通过了占位符完整性校验（渲染后无 { } 残留）？
□ 降级链是否仍完整（专用 → 通用战术 → 通用着法 → 兜底）？
□ 评价阈值边界（9/10/11、49/50/51、149/150/151、399/400/401）是否仍被覆盖？
□ 若改动了阈值：是否同步回写了 docs/01 §4.3 与 docs/05 §4.2？
□ analyze 永不 panic 的随机测试是否仍通过（10000 次）？
□ 素材 JSON 变更是否通过了构建期 schema 校验（docs/05 §9.4）？
```

### 10.5 服务端（`xq-server`）专项

```
□ 所有着法是否都经过服务端 xq-core 裁决？（不得信任客户端提交的任何状态）
□ 新增端点是否有鉴权、限流、输入校验（长度/范围/格式）？
□ 新增 Redis 键是否有 TTL（Stream 除外，用 MAXLEN）？
□ 新增配置项是否三处同步（default.toml / .env.example / 设计文档）？
□ 数据库变更是否通过 migrations/？是否有回滚路径？
□ 新增指标/日志是否带全 request_id / room_id / game_id？
□ 是否有 N+1 查询？新增查询是否验证了执行计划（≥ 万行表的查询）？
□ 事务边界是否正确（战绩 + 积分 + 流水必须在同一事务，见 docs/06 §4.3 T10）？
```

### 10.6 客户端（Tauri + 前端）专项

```
□ 耗时 > 16 ms 的计算是否投递到了引擎线程池（不得阻塞 WebView 主线程）？
□ 组件内是否直接 invoke？（必须经 src/api 层）
□ 坐标换算相关改动是否附了 90 格全覆盖测试？
□ 新增依赖是否评估了体积（前端体积预算 ≤ 2.5 MB gzip）？
□ 动画是否只使用 transform / opacity？（不得触发 layout）
□ 是否处理了键盘操作与 prefers-reduced-motion？
□ 客户端是否引入了任何密钥？（NFR-S.09，绝对禁止）
□ 离线路径是否仍然可用（断网时人机对战 + 本地讲解）？
```

### 10.7 通用项

```
□ 提交标题符合 Conventional Commits，scope 正确
□ 无新增 TODO/FIXME（或已带里程碑标记）
□ 无注释掉的死代码、无 dbg!/console.log
□ 新依赖的必要性是否说明？（能否用已有依赖或标准库替代？）
□ 是否评估了安全影响（注入 / 越权 / 限流绕过）？
□ 日志中未出现密码 / token / 完整手机号（§9.3 红线）
□ 设计文档中声明的数值 / 契约若被改动，是否在**同一 PR** 内更新了文档？
□ 新增文档链接使用相对路径
□ 回滚方式是否明确？（revert / feature flag / 迁移回滚）
```

### 10.8 Reviewer 结论模板

```markdown
## 评审结论

**阻塞项（必须修改）**
- [ ] （若有）

**建议项（可后续跟进）**
- [ ] （若有）

**已确认的关键点**
- 规则正确性：已复核 docs/03 §x.x 与测试用例的对应关系
- 约束符合性：已确认无 IO 依赖 / 无密钥 / 日志合规
- 性能：已复核 criterion 对比（劣化 x%）

**结论**：Request Changes / Approve
```

---

## 附录 A：本文档中的「估算值 / 待校准项」清单

> 按硬约束要求，以下均为**设计初始值**或**估算**，未经实测/校准前不得作为验收结论或对外承诺。

| 编号 | 项 | 本文取值 | 性质 | 校准方式 |
|---|---|---|---|---|
| E-01 | CI 各阶段耗时 | 1~40 min | **估算** | 首次跑通 CI 后记录实际值 |
| E-02 | 领域层直接依赖数上限 | core 8 / ai 10 / coach 12 | 设计初始值 | M0 依赖定型后按实际值校准 |
| E-03 | 前端体积预算 | dist ≤ 2.5 MB gzip；首屏 ≤ 400 KB | 设计初始值 | 首次构建后实测，对齐 NFR-P.09/08 |
| E-04 | 基准回归阈值 | core 15% / ai 20% / 其他 10% | 部分有依据（ai 20% 来自 [docs/04 §8.3](04-AI引擎设计.md)） | 基线稳定后收紧或放宽 |
| E-05 | 覆盖率门禁 | NFR-M.02 的 85% / 95% | 有依据（NFR-M.02） | 首次覆盖率报告后确认可达性 |
| E-06 | Windows 交叉编译方案 | 待定（msvc 交叉 vs 原生） | 待实测 | M0 期间实测两种方案 |
| E-07 | 里程碑→版本映射 | M1→0.2.0 等 | 建议值 | 产品确认 |
| E-08 | macOS 公证耗时预算 | ≥ 60 min 超时 | 经验估值 | 首次公证实测 |

## 附录 B：与其他文档的接口

| 本文的约定 | 消费方 |
|---|---|
| `assert_no_io_deps.sh`（ADR-005 强制点） | [docs/02 §2.1 不变量 I-1](02-系统架构设计.md) |
| 错误消息红线（CI 断言） | [docs/10 §12 C-07](10-API接口规范.md) |
| 公开 API 快照 | [docs/02 §2.1 不变量 I-2](02-系统架构设计.md) |
| 覆盖率门禁阈值 | [docs/01 NFR-M.02](01-需求规格说明书.md) |
| 基准回归阈值 | [docs/03 §12](03-规则引擎与领域模型.md)、[docs/04 §8.3](04-AI引擎设计.md) |
| 日志必带字段 | [docs/02 §9](02-系统架构设计.md)、[NFR-M.04](01-需求规格说明书.md) |
| 配置分层与 `XQ__` 前缀 | [docs/02 §8](02-系统架构设计.md) |
| 三端打包矩阵与系统依赖 | [docs/08 §15](08-客户端设计.md) |
| 测试阶段划分（哪些进 nightly） | [docs/12](12-测试策略与质量保障.md) |
| 里程碑与质量门禁的关系 | [docs/13](13-路线图与里程碑.md) |
