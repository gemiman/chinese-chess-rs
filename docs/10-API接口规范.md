# 10 · API 接口规范

| 项 | 值 |
|---|---|
| 所属模块 | `xq-server::routes`（REST 处理器）· `xq-protocol`（DTO 与错误码契约） |
| 文档版本 | v1.0 |
| 状态 | 设计基线（Design Baseline） |
| 编制日期 | 2026-09-23 |
| 关联文档 | [01-需求规格](01-需求规格说明书.md) · [02-系统架构](02-系统架构设计.md) · [03-规则引擎](03-规则引擎与领域模型.md) · [05-战法讲解](05-战法讲解引擎设计.md) · [06-联网协议](06-联网对战与实时通信协议.md) · [09-数据模型](09-数据模型与存储设计.md) · [ADR-003](14-决策记录ADR.md#adr-003) · [ADR-007](14-决策记录ADR.md#adr-007) · [ADR-010](14-决策记录ADR.md#adr-010) |

> **本文的定位**：定义客户端与 `xq-server` 之间**所有 REST 契约**。DTO 类型定义在 `xq-protocol` crate 中（[02 §3.2](02-系统架构设计.md)），因此本文的字段名与类型**即** Rust 结构体字段名——文档与代码不得出现偏差。
>
> **范围说明**：实时帧（走子、观战、匹配推送、讲解推送）由 [06-联网对战与实时通信协议](06-联网对战与实时通信协议.md) 定义，本文只在 §7 明确职责边界。
>
> **数字口径声明**：本文所有有效期、限流阈值、分页大小均为**设计初始值**，带 ⚠️ 标记的项**未经实测或校准**。§12 汇总了全部待校准项。

---

## 1. 总览与约定

### 1.1 全局约定

| 项 | 约定 |
|---|---|
| Base Path | **`/api/v1`**（运行环境可加部署前缀，如 `/xq/api/v1`；客户端应从配置读取，不硬编码） |
| 内容类型 | 请求与响应均为 `application/json; charset=utf-8`。**例外**：棋谱导出返回 `text/plain; charset=utf-8`（§3.5） |
| 字符编码 | 强制 UTF-8。所有响应显式带 `charset=utf-8`（中文字段必须无乱码，NFR-C.06） |
| 时间格式 | **RFC 3339，UTC，毫秒精度**，固定以 `Z` 结尾：`2026-09-23T10:15:30.123Z`。**不做本地化**，客户端自行按用户时区显示 |
| ID 格式 | **UUID v7 的 36 字符小写字符串**（如 `018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f23`）。理由见 §1.4 |
| 命名风格 | JSON 字段、查询参数、自定义请求头一律 `snake_case`；与 [09 §3](09-数据模型与存储设计.md) 的列名保持同名（如 `time_per_move_ms`、`score_loss`） |
| 枚举值 | 小写蛇形字符串，与 PostgreSQL 枚举**逐字对应**（[09 §3.0](09-数据模型与存储设计.md)）：`red_win`、`checkmate`、`dubious`、`ranked` |
| 布尔 | JSON `true` / `false`（不用 `0/1`，不用 `"true"`） |
| 可空 | 可选字段可缺省或为 `null`。**不接受**用空字符串表达空值 |
| 成功响应结构 | 直接返回资源对象（`{...}`）或列表包装（`{items, ...}`）；**不套 `data` 外壳** |
| 错误响应结构 | 统一 `{"error": {...}}`，见 §4.1 |
| 时间戳语义 | `created_at` = 记录创建时间；`started_at` / `ended_at` = 对局开局/结束时间（[09 §3.4](09-数据模型与存储设计.md)） |
| 分页 | 对局列表用**游标**，排行榜用**偏移**（§5） |
| API 版本 | 路径版本 `/api/v1`，兼容策略见 §9 |

### 1.2 通用请求头

| 请求头 | 必需性 | 说明 |
|---|---|---|
| `Authorization` | 需鉴权端点必需 | `Bearer <access_token>`。**仅**此一种鉴权方式 |
| `Content-Type` | 有请求体时必需 | `application/json`（`charset` 可省略，服务端按 UTF-8 处理） |
| `Accept` | 否 | 仅支持 `application/json`；其他值不报错，仍返回 JSON |
| `X-Request-Id` | 否 | 客户端生成的追踪 ID（UUID v7 字符串）。缺省时服务端生成。**服务端必须回写同名响应头**（[02 §9](02-系统架构设计.md) 全链路上下文） |
| `Idempotency-Key` | 幂等端点建议携带 | UUID v7 字符串，见 §8 |

### 1.3 通用响应头

| 响应头 | 出现条件 | 说明 |
|---|---|---|
| `X-Request-Id` | 所有响应 | 与请求头同名；缺省时服务端生成 |
| `X-RateLimit-Limit` | 所有受限流端点 | 当前窗口的配额上限（整数） |
| `X-RateLimit-Remaining` | 所有受限流端点 | 剩余可用次数（整数） |
| `X-RateLimit-Reset` | 所有受限流端点 | 窗口重置时刻（**Unix 秒**，与 RFC 3339 时间字段区分，此头遵循业界惯例） |
| `Retry-After` | `429` / `503` | 建议重试等待秒数（整数） |
| `Deprecation` / `Sunset` / `Link` | 端点被废弃时 | 见 §9.2 |
| `Cache-Control` | 鉴权相关响应 | `no-store`（`/auth/*` 与 `/users/me*`） |

### 1.4 为什么 ID 用字符串传递

| 论点 | 说明 |
|---|---|
| **JS 数字精度上限** | `Number.MAX_SAFE_INTEGER = 2^53 − 1 = 9,007,199,254,740,991`。JSON 无反引号整数类型，`JSON.parse` 遇到超过该值的整数会**静默丢精度**（不报错），导致"前端显示的 ID 与后端不是同一个" |
| **内部存在 bigint 主键** | `game_moves.id`、`game_coach_notes.id`、`rating_history.id`、`audit_logs.id`、`match_queue_logs.id` 都是 `bigint GENERATED ALWAYS AS IDENTITY`（[09 §3](09-数据模型与存储设计.md)）。`game_moves` 单表在 1 万 DAU 下约 1.3 × 10⁸ 行/年，**约 1 年即可逼近 2^53** |
| **契约层隔离** | **所有 `bigint` 自增主键一律不对外暴露**。对外标识只用 UUID v7（`users.id` / `games.id` / `reports.id` / `seasons.id`） |
| **UUID 为何用 v7 而非 v4** | v7 前 48 位为毫秒时间戳，**新 ID 单调递增** → B-tree 插入落在最右侧页，无随机插入的页分裂与写放大；且天然带创建时间，可直接用于排序与游标（[README §3.2](../README.md)） |
| **类型一致的代价** | `game_moves` 对外是否暴露 ID？**否**。着法用 `(game_id, seq)` 复合定位，天然是字符串 + 整数，无精度问题（[09 §3.5](09-数据模型与存储设计.md)）。因此本项目**不存在"必须暴露大整数"的场景** |

> **实现约束**：`xq-protocol` 中所有对外 ID 字段类型为 `String`（或 `Uuid` + `serde` 序列化为字符串），**禁止**使用 `i64` 承载对外 ID。这条约束应由 `xq-protocol` 的单元测试（手写 `Deserialize` 断言）锁定。

---

## 2. 鉴权

### 2.1 Token 体系

| 项 | **access token** | **refresh token** |
|---|---|---|
| 形态 | JWT（`jsonwebtoken` 11.1.0） | JWT |
| 签名算法 | **EdDSA（Ed25519）** 非对称签名（NFR-S.06 "非对称签名或高熵密钥"）；单实例简化部署可配置为 HS256，密钥 ≥ 32 字节随机 ⚠️ 部署时确认 | 同左 |
| 有效期 | **900 秒（15 分钟）** ⚠️ 设计初始值 | **2,592,000 秒（30 天）** ⚠️ 设计初始值，**绝对有效期**（不随刷新延长） |
| 传输方式 | `Authorization: Bearer <token>` 请求头 | 请求体字段 `refresh_token`（**不放请求头**，避免被代理/日志记录） |
| 服务端存储 | Redis **黑名单**：`session:access:deny:{jti}`（**仅登出时写入**，TTL = 剩余有效期） | Redis **白名单**：`session:refresh:{jti}` = `user_id`（登录即写入，TTL = 30 天） |
| 每请求校验成本 | 1 次 `GET`（黑名单 miss 即通过） | 1 次 `GET`（白名单必须命中） |
| 客户端存储 | 内存（**不落盘**） | 操作系统凭据存储（Windows Credential Manager / macOS Keychain / Linux Secret Service）。**禁止**写明文文件（NFR-S.09） |
| 泄露影响 | ≤ 15 分钟（除非同时泄露 refresh） | 30 天；由**轮换 + 复用检测**缓解（§2.3） |

**Access token Claims**

```json
{
  "iss": "xq-server",
  "aud": "xq-client",
  "sub": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f23",
  "jti": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f24",
  "typ": "access",
  "role": "user",
  "ver": 1,
  "iat": 1758623730,
  "exp": 1758624630
}
```

**Refresh token Claims**：同上，但 `typ = "refresh"`，`exp = iat + 2592000`。

| Claim | 说明 |
|---|---|
| `sub` | 用户 ID（UUID v7 字符串） |
| `jti` | Token 唯一 ID（UUID v7），**黑/白名单的键** |
| `typ` | `access` / `refresh`。**端点必须校验类型**：`/auth/refresh` 只接受 `typ = "refresh"`，其余需鉴权端点只接受 `typ = "access"` |
| `role` | `user` / `admin`（运营端点用） |
| `ver` | Token 版本号。密钥轮换或"登出所有设备"时递增，使全部旧 token 失效（[09 §5.2](09-数据模型与存储设计.md) `session:user` 的兜底方案） |

**校验规则（必须全部满足）**

| 规则 | 理由 |
|---|---|
| `alg` 必须**严格等于**配置算法 | 防止算法混淆攻击（如 `alg: none`、把 RS256 降级为 HS256 并用公钥当 HMAC 密钥） |
| `iss` / `aud` 必须匹配配置值 | 防止跨系统 token 混用 |
| `exp` 校验容忍 **30 秒**时钟偏移 | service 与校验方时钟漂移；⚠️ 30 s 为设计初始值 |
| `typ` 必须与端点期望一致 | 防止用 refresh token 直接访问业务端点（会被 refresh 白名单放过，但业务端点不查白名单） |
| `ver` 必须等于当前有效版本 | 支持"全量失效" |
| `jti` 不在 `session:access:deny:*` 中 | 支持登出立即生效（窗口 ≤ 15 分钟） |

### 2.2 鉴权要求分级

| 级别 | 含义 | 端点 |
|---|---|---|
| **公开** | 不携带 `Authorization` 也可访问 | `GET /healthz`、`GET /readyz`、`POST /auth/register`、`POST /auth/login`、`POST /auth/refresh`、`GET /leaderboard`、`GET /lobby/games`、`GET /rooms/{room_code}`、`GET /seasons/current` |
| **可选鉴权** | 携带则返回额外字段（如"是否是我"、"我是否已点赞"），不携带也可访问 | `GET /users/{user_id}`、`GET /games/{game_id}`、`GET /games/{game_id}/coach-notes` |
| **必需鉴权** | 缺失 → `401 AUTH001`；`typ ≠ access` → `401 AUTH001`；token 过期 → `401 AUTH002` | 其余全部端点 |
| **管理员** | 额外校验 `role = "admin"`，否则 `403 AUTH010` | 本期**不定义**运营后台 REST 端点（举报处理走内部工具，[01 §3](01-需求规格说明书.md) 角色表已列 Admin 但未列入本期 REST 范围） |

> **游客（Guest）说明**：游客的人机对战与本地讲解完全在客户端完成，**不访问服务端**（FR-08.01、NFR-R.06）。因此 REST 层**不提供游客 token**。游客点击"联网对战"时客户端应引导登录，服务端返回 `401 AUTH001` 即可。

### 2.3 刷新流程（轮换 + 复用检测）

```mermaid
sequenceDiagram
    participant C as 客户端
    participant S as xq-server
    participant R as Redis

    Note over C: access token 过期（401 AUTH002）
    C->>S: POST /api/v1/auth/refresh {refresh_token}
    S->>S: 校验签名 / exp / typ=refresh / ver
    S->>R: GET session:refresh:{jti}
    alt 白名单命中
        R-->>S: user_id
        S->>R: DEL session:refresh:{旧 jti}
        S->>R: SREM session:user:{uid} {旧 jti}
        S->>R: SET session:refresh:{新 jti} {uid} EX 2592000
        S->>R: SADD session:user:{uid} {新 jti}
        S-->>C: {access_token(新), refresh_token(新), expires_in: 900}
    else 白名单未命中（token 已用/已注销/已过期）
        S->>R: SMEMBERS session:user:{uid}
        S->>R: DEL session:refresh:{该用户全部 jti}
        S->>S: 记录 audit_logs(action='auth.refresh_reuse', result='failure')
        S-->>C: 401 AUTH005
    end
```

**轮换（Rotation）**：每次刷新**必然签发新的 refresh token**，旧的立即失效。
**复用检测（Reuse Detection）**：若一个已被轮换掉的 refresh token 再次被使用，视为**token 泄露信号** → **撤销该用户全部 refresh token**（强制全部设备重新登录），并写审计日志（`action = 'auth.refresh_reuse'`）。

| 参数 | 取值 | 说明 |
|---|---|---|
| access 有效期 | 900 s ⚠️ 设计初始值 | 依据：短到"泄露后损失可忽略"，长到"不必每秒刷新"。15 分钟是业界常见量级 |
| refresh 有效期 | 2,592,000 s（30 天）⚠️ 设计初始值 | 桌面客户端属于"低频使用"，30 天避免频繁登录 |
| 客户端提前刷新窗口 | 到期前 **60 s** ⚠️ 设计初始值 | 避免"正好在请求途中过期"导致的 `AUTH002` |
| 并发刷新保护 | 客户端需实现**单飞（single-flight）**：同一时刻只允许一个刷新请求在途，其余请求等待其结果 | 否则 10 个并发请求会触发 10 次刷新 → 9 次命中"复用检测" → 误判为泄露并踢出全部设备。**这是必须由客户端实现的硬要求** |

### 2.4 登出与失效

| 操作 | 端点 | 服务端动作 |
|---|---|---|
| 登出当前设备 | `POST /auth/logout` | ① `DEL session:refresh:{refresh_jti}`；② `SREM session:user:{uid} {refresh_jti}`；③ `SET session:access:deny:{access_jti} 1 EX <access 剩余秒数>`；④ 写审计 |
| 登出全部设备 | `POST /auth/logout` with `{"all_devices": true}` | ① `SMEMBERS session:user:{uid}` 逐个 `DEL session:refresh:{jti}`；② `DEL session:user:{uid}`；③ 递增 `ver`（见下）；④ 写审计 |
| 修改密码 | `POST /auth/password` | 同"登出全部设备"（改密即全端下线，安全基线），并递增 `ver` |

**`ver` 递增的实现方式**：`ver` 需要持久化才能跨实例与重启生效，而 Redis 中无对应字段。**设计选择**：在 `users` 表增加 `token_version integer NOT NULL DEFAULT 1` 列，递增时 `UPDATE users SET token_version = token_version + 1`（⚠️ **该列需补入 [09 §3.1](09-数据模型与存储设计.md) 的 `users` 表定义**，属本文对 09 的接口需求；本文不直接修改 09 文件，已在 [09 §10.5](09-数据模型与存储设计.md) 的 C-20 项登记）。

> 若不引入 `token_version` 列，替代方案是把当前版本写入 Redis 键 `session:version:{user_id}`（无 TTL 则与 [ADR-010](14-决策记录ADR.md#adr-010)"所有键必须有 TTL"冲突，故不推荐）。**结论：采用 `users.token_version` 列。**

### 2.5 密码策略

| 规则 | 取值 | 依据 |
|---|---|---|
| 长度 | **8 ≤ len ≤ 64** ⚠️ 设计初始值 | 下限 8 是业界基线；上限 64 防止"超长密码做 DoS"（Argon2id 计算量与输入长度弱相关，但传输与日志需要边界） |
| 字符类型 | 至少包含 **2** 类：小写字母 / 大写字母 / 数字 / 符号 ⚠️ 设计初始值 | 不应过严（会推高用户放弃率），2 类 + 长度 8 + 弱密码表已是合理基线 |
| 弱密码表 | 内置 ≥ 10,000 条常见弱密码，命中即拒绝 ⚠️ 表规模待定 | 比复杂度规则更有效（`Password1` 满足复杂度但极弱） |
| 与用户名/昵称的关系 | 不得包含用户名，不得与昵称相同（大小写不敏感） | 降低定向猜测成功率 |
| 哈希算法 | **Argon2id**（`argon2` crate，[README §3.2](../README.md)） | NFR-S.05 硬要求 |
| 哈希参数 | `m = 19456 KiB（19 MiB）`、`t = 2`、`p = 1` ⚠️ **必须实测**：目标单次哈希 **50~100 ms**（同一硬件上），超目标需下调 `m` | 参考 OWASP《密码存储备忘单》的 Argon2id 推荐配置。**该参数组合下的实际耗时依硬件而异，必须实测后固化** |
| 存储形式 | PHC 字符串（如 `$argon2id$v=19$m=19456,t=2,p=1$...`）写入 `users.password_hash` | 自带参数与盐，便于未来升级参数 |
| 时序攻击 | 用户不存在时**仍执行一次 Argon2id 校验**（对固定假哈希），使响应时间与"用户存在但密码错"一致 | 防止通过响应时间枚举用户名 |
| 失败计数 | 密码错一次 `failed_login_count += 1`；成功则归零；连续 **5 次** → `locked_until = now() + 15 min` ⚠️ 阈值与时长均为设计初始值 | 见 §6 登录限流 |

---

## 3. REST 端点详表

> **阅读约定**：每个端点给出「鉴权 / 幂等 / 限流」三项元信息、请求参数表、请求示例、响应示例、错误码。请求与响应示例均为**完整可解析的 JSON**，字段名与 `xq-protocol` 的 DTO 一致。
> 路径中的 `{...}` 为路径参数。所有列表字段返回**数组**（可能为空数组，不返回 `null`）。

### 3.1 账号域 `/api/v1/auth`

#### 3.1.1 注册

| 项 | 值 |
|---|---|
| 方法 / 路径 | `POST /api/v1/auth/register` |
| 鉴权 | 公开 |
| 幂等 | 天然幂等（`users.username` 唯一索引；重复提交返回 `AUTH008`） |
| 限流 | IP 维度 **5 次/小时** |

**请求参数**

| 字段 | 类型 | 必需 | 约束 | 说明 |
|---|---|---|---|---|
| `username` | string | 是 | 3..32，`^[a-z0-9_]+$`（服务端会先转小写，但**拒绝**含大写以免歧义） | 登录名 |
| `password` | string | 是 | 见 §2.5 | 密码明文（**仅 HTTPS 传输**，服务端不落日志） |
| `nickname` | string | 是 | 1..24，不含控制字符与前后空白 | 展示昵称 |
| `avatar_id` | string | 否 | 必须在内置头像集合中 | 缺省使用默认头像 |

**请求示例**

```json
POST /api/v1/auth/register
Content-Type: application/json

{
  "username": "red_master",
  "password": "Xq!2026play",
  "nickname": "红方大师",
  "avatar_id": "preset_07"
}
```

**响应示例**（`201 Created`）

```json
{
  "user": {
    "id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f23",
    "username": "red_master",
    "status": "active",
    "created_at": "2026-09-23T10:15:30.123Z"
  },
  "profile": {
    "nickname": "红方大师",
    "avatar_id": "preset_07",
    "rating": 1500,
    "peak_rating": 1500,
    "wins": 0,
    "losses": 0,
    "draws": 0,
    "total_games": 0
  },
  "access_token": "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...",
  "refresh_token": "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...",
  "token_type": "Bearer",
  "expires_in": 900
}
```

> 立即签发 token：避免"注册后还要再登录一次"的多余往返。`expires_in` 单位是**秒**。

**错误码**：`AUTH003`（密码强度不足）、`AUTH008`（用户名已存在）、`SYS004`（参数校验失败）、`RATE001`

---

#### 3.1.2 登录

| 项 | 值 |
|---|---|
| 方法 / 路径 | `POST /api/v1/auth/login` |
| 鉴权 | 公开 |
| 幂等 | 不适用（每次登录产生新 token 对） |
| 限流 | **双维度**：账号维度 **10 次/5 分钟**；IP 维度 **30 次/5 分钟**（见 §6） |

**请求参数**

| 字段 | 类型 | 必需 | 说明 |
|---|---|---|---|
| `username` | string | 是 | 登录名 |
| `password` | string | 是 | 密码 |

**请求示例**

```json
POST /api/v1/auth/login
Content-Type: application/json

{ "username": "red_master", "password": "Xq!2026play" }
```

**响应示例**（`200 OK`）

```json
{
  "user": {
    "id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f23",
    "username": "red_master",
    "status": "active",
    "created_at": "2026-09-23T10:15:30.123Z"
  },
  "profile": {
    "nickname": "红方大师",
    "avatar_id": "preset_07",
    "rating": 1524,
    "peak_rating": 1536,
    "wins": 12,
    "losses": 5,
    "draws": 1,
    "total_games": 18
  },
  "access_token": "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...",
  "refresh_token": "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...",
  "token_type": "Bearer",
  "expires_in": 900
}
```

**错误码**：`AUTH004`（用户名或密码错误）、`AUTH006`（账号被停用）、`AUTH007`（账号已注销）、`AUTH009`（登录尝试过于频繁）、`RATE001`

> **安全约定**：`AUTH004` 的 `message` **不区分**"用户名不存在"与"密码错误"（统一文案"用户名或密码错误"），防止用户名枚举。服务端日志中保留区分（便于运营排查）。

---

#### 3.1.3 刷新 Token

| 项 | 值 |
|---|---|
| 方法 / 路径 | `POST /api/v1/auth/refresh` |
| 鉴权 | 公开（凭 `refresh_token` 自身鉴权） |
| 幂等 | **否**（每次刷新都轮换 token，重复提交会触发复用检测） |
| 限流 | 用户维度 **60 次/小时**；IP 维度 **200 次/小时** |

**请求参数**

| 字段 | 类型 | 必需 | 说明 |
|---|---|---|---|
| `refresh_token` | string | 是 | 上一次登录/刷新返回的 refresh token |

**请求示例**

```json
POST /api/v1/auth/refresh
Content-Type: application/json

{ "refresh_token": "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9..." }
```

**响应示例**（`200 OK`）

```json
{
  "access_token": "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...",
  "refresh_token": "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...",
  "token_type": "Bearer",
  "expires_in": 900
}
```

**错误码**：`AUTH002`（refresh token 过期）、`AUTH005`（refresh token 无效或已失效——含已注销、已被轮换、复用检测命中）、`AUTH006`、`AUTH007`、`RATE001`

---

#### 3.1.4 登出

| 项 | 值 |
|---|---|
| 方法 / 路径 | `POST /api/v1/auth/logout` |
| 鉴权 | **必需**（需 access token 以取其 `jti` 入黑名单） |
| 幂等 | **是**（重复登出均返回 `204`；见 §8.2） |
| 限流 | 用户维度 **30 次/小时** |

**请求参数**

| 字段 | 类型 | 必需 | 默认 | 说明 |
|---|---|---|---|---|
| `refresh_token` | string | 否 | — | 提供则精确删除该 refresh token 的白名单；不提供则仅黑名单当前 access token |
| `all_devices` | boolean | 否 | `false` | `true` 时撤销该用户全部 refresh token 并递增 `token_version` |

**请求示例**

```json
POST /api/v1/auth/logout
Authorization: Bearer eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...
Content-Type: application/json
Idempotency-Key: 018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f30

{
  "refresh_token": "eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...",
  "all_devices": false
}
```

**响应示例**：`204 No Content`（无响应体）

**错误码**：`AUTH001`、`AUTH002`、`RATE001`

---

#### 3.1.5 修改密码

| 项 | 值 |
|---|---|
| 方法 / 路径 | `POST /api/v1/auth/password` |
| 鉴权 | **必需** |
| 幂等 | **否**（改密是有副作用的操作，且会导致全端下线；重复提交第二次会因旧密码已变而失败） |
| 限流 | 用户维度 **5 次/小时** |

**请求参数**

| 字段 | 类型 | 必需 | 说明 |
|---|---|---|---|
| `old_password` | string | 是 | 当前密码（**必须校验**，防止会话被劫持后直接改密） |
| `new_password` | string | 是 | 见 §2.5；**不得与 `old_password` 相同** |

**请求示例**

```json
POST /api/v1/auth/password
Authorization: Bearer eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...
Content-Type: application/json

{ "old_password": "Xq!2026play", "new_password": "Xq!2026play#2" }
```

**响应示例**（`200 OK`）

```json
{
  "changed_at": "2026-09-23T10:20:00.000Z",
  "sessions_revoked": 3,
  "message": "密码已修改，其他设备的登录状态已全部失效。"
}
```

**错误码**：`AUTH001`、`AUTH002`、`AUTH003`（新密码强度不足）、`AUTH004`（原密码错误，复用同一码表达"凭据错误"）、`RATE001`

> 改密后**必须**全端下线（`sessions_revoked` 报告被撤销的会话数），这是安全基线。

---

### 3.2 资料域 `/api/v1/users`

#### 3.2.1 获取我的资料

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /api/v1/users/me` |
| 鉴权 | **必需** |
| 幂等 | 是（安全方法） |
| 限流 | 用户维度 **120 次/分钟** |
| 缓存 | `Cache-Control: no-store` |

**请求参数**：无

**请求示例**

```
GET /api/v1/users/me
Authorization: Bearer eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...
```

**响应示例**（`200 OK`）

```json
{
  "user": {
    "id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f23",
    "username": "red_master",
    "status": "active",
    "created_at": "2026-09-23T10:15:30.123Z"
  },
  "profile": {
    "nickname": "红方大师",
    "avatar_id": "preset_07",
    "rating": 1524,
    "peak_rating": 1536,
    "wins": 12,
    "losses": 5,
    "draws": 1,
    "total_games": 18,
    "aborted_games": 0
  },
  "season": {
    "season_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f40",
    "name": "2026-S4",
    "seq": 4,
    "rating": 1524,
    "peak_rating": 1536,
    "wins": 12,
    "losses": 5,
    "draws": 1,
    "final_rank": null,
    "honor": null
  }
}
```

> `aborted_games`（中途退出局数）**仅对本人返回**，不在 `GET /users/{user_id}` 中暴露（避免公开羞辱标记）。

**错误码**：`AUTH001`、`AUTH002`、`SYS001`

---

#### 3.2.2 获取他人资料

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /api/v1/users/{user_id}` |
| 鉴权 | **可选**（携带鉴权且查看自己时，行为等同 `/users/me` 的裁剪版） |
| 幂等 | 是 |
| 限流 | 用户/IP 维度 **120 次/分钟** |

**路径参数**

| 参数 | 类型 | 说明 |
|---|---|---|
| `user_id` | string | 目标用户的 UUID v7 字符串 |

**请求示例**

```
GET /api/v1/users/018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f23
```

**响应示例**（`200 OK`）

```json
{
  "user": {
    "id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f23",
    "status": "active",
    "created_at": "2026-09-23T10:15:30.123Z"
  },
  "profile": {
    "nickname": "红方大师",
    "avatar_id": "preset_07",
    "rating": 1524,
    "peak_rating": 1536,
    "wins": 12,
    "losses": 5,
    "draws": 1,
    "total_games": 18
  },
  "is_self": false
}
```

> **隐私约定**：不返回 `username`（登录名属账号信息，非公开资料）、不返回 `aborted_games`、不返回 `status` 中的 `suspended` 细节（`status` 统一返回 `"active"` 或 `"deleted"`）。

**错误码**：`USER001`（用户不存在）、`SYS001`

---

#### 3.2.3 更新昵称 / 头像

| 项 | 值 |
|---|---|
| 方法 / 路径 | `PATCH /api/v1/users/me` |
| 鉴权 | **必需** |
| 幂等 | **是**（PATCH 语义为"设为该值"，重复提交结果相同，见 §8.2） |
| 限流 | 用户维度 **10 次/小时** |
| 请求体大小 | ≤ 1 KiB |

**请求参数**（全部可选，但至少提供一个）

| 字段 | 类型 | 约束 | 说明 |
|---|---|---|---|
| `nickname` | string | 1..24，非空，不含控制字符；大小写不敏感唯一 | 昵称 |
| `avatar_id` | string | 必须在内置头像集合 `preset_01`..`preset_N` 中 | 头像标识 |

**请求示例**

```json
PATCH /api/v1/users/me
Authorization: Bearer eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...
Content-Type: application/json

{ "nickname": "楚河大师", "avatar_id": "preset_12" }
```

**响应示例**（`200 OK`）

```json
{
  "profile": {
    "nickname": "楚河大师",
    "avatar_id": "preset_12",
    "rating": 1524,
    "peak_rating": 1536,
    "wins": 12,
    "losses": 5,
    "draws": 1,
    "total_games": 18,
    "aborted_games": 0
  },
  "updated_at": "2026-09-23T10:25:00.000Z"
}
```

**错误码**：`AUTH001`、`AUTH002`、`USER002`（昵称已被占用）、`USER003`（头像标识无效）、`USER004`（昵称不符合规范）、`SYS004`、`RATE001`

---

### 3.3 战绩域 `/api/v1/games`

#### 3.3.1 我的对局列表

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /api/v1/games` |
| 鉴权 | **必需**（只能查自己的对局） |
| 幂等 | 是 |
| 限流 | 用户维度 **60 次/分钟** |
| 分页 | **游标分页**（§5.1） |

**查询参数**

| 参数 | 类型 | 必需 | 默认 | 约束 | 说明 |
|---|---|---|---|---|---|
| `cursor` | string | 否 | — | 不透明字符串 | 上一页返回的 `next_cursor` |
| `limit` | integer | 否 | `20` | 1..100 | 每页条数 |
| `status` | string | 否 | — | `ongoing` \| `finished` \| `aborted` | 按状态过滤 |
| `mode` | string | 否 | — | `ranked` \| `room` \| `casual` | 按模式过滤 |
| `result` | string | 否 | — | `win` \| `loss` \| `draw` | **以当前用户视角**过滤（服务端据 `red_user_id` / `black_user_id` 与 `winner` 换算） |
| `opponent_id` | string | 否 | — | UUID v7 | 只看与某人的对局 |
| `season_id` | string | 否 | — | UUID v7 | 只看某个赛季 |

**请求示例**

```
GET /api/v1/games?limit=20&status=finished&result=win
Authorization: Bearer eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...
```

**响应示例**（`200 OK`）

```json
{
  "items": [
    {
      "id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f50",
      "room_code": "482913",
      "mode": "ranked",
      "status": "finished",
      "rules_version": "2026.1",
      "season_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f40",
      "my_side": "red",
      "red": {
        "user_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f23",
        "nickname": "楚河大师",
        "rating_before": 1512,
        "rating_delta": 12,
        "is_ai": false
      },
      "black": {
        "user_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f24",
        "nickname": "汉界棋手",
        "rating_before": 1508,
        "rating_delta": -12,
        "is_ai": false
      },
      "result": "red_win",
      "winner": "red",
      "end_reason": "checkmate",
      "my_result": "win",
      "total_plies": 74,
      "time_total_ms": 600000,
      "time_per_move_ms": 30000,
      "started_at": "2026-09-23T09:40:00.000Z",
      "ended_at": "2026-09-23T09:58:22.410Z",
      "created_at": "2026-09-23T09:39:55.000Z"
    }
  ],
  "next_cursor": "eyJ0IjoxNzU4NjIzNzMwMDAwLCJpIjoiMDE4ZjJhM2MifQ",
  "has_more": true
}
```

> **`my_side` / `my_result` 字段**：服务端按当前用户视角计算，避免客户端自行判断"我是红还是黑"而引入不一致。
> **列表不返回着法与讲解**（响应体控制），需要时调用 §3.3.2 / §3.3.3。

**错误码**：`AUTH001`、`AUTH002`、`SYS004`（`cursor` 或 `limit` 非法）、`SYS001`

---

#### 3.3.2 对局详情（含全部着法）

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /api/v1/games/{game_id}` |
| 鉴权 | **可选**（公开对局匿名可查；私密对局需参与方，否则 `403 GAME005`） |
| 幂等 | 是 |
| 限流 | 用户/IP 维度 **120 次/分钟** |
| 响应体大小 | 典型 ≤ 40 KiB（160 着），上限 **256 KiB** |

**查询参数**

| 参数 | 类型 | 必需 | 默认 | 说明 |
|---|---|---|---|---|
| `include` | string | 否 | — | 逗号分隔。可选 `moves`（默认含）、`stats`（含 `move_stats` 汇总）、`fens`（**含着法后 FEN**，显著增大响应体，见下） |

**请求示例**

```
GET /api/v1/games/018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f50?include=moves,stats
```

**响应示例**（`200 OK`，为控制篇幅省略部分着法）

```json
{
  "id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f50",
  "room_code": "482913",
  "mode": "ranked",
  "status": "finished",
  "rules_version": "2026.1",
  "season_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f40",
  "red": {
    "user_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f23",
    "nickname": "楚河大师",
    "rating_before": 1512,
    "rating_delta": 12,
    "is_ai": false,
    "time_left_ms": 412300
  },
  "black": {
    "user_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f24",
    "nickname": "汉界棋手",
    "rating_before": 1508,
    "rating_delta": -12,
    "is_ai": false,
    "time_left_ms": 288940
  },
  "result": "red_win",
  "winner": "red",
  "end_reason": "checkmate",
  "total_plies": 74,
  "initial_fen": "rnbakabnr/9/1c5c1/p1p1p1p1p/9/9/P1P1P1P1P/1C5C1/9/RNBAKABNR w - - 0 1",
  "final_fen": "r1bakabnr/9/1cn4c1/p1p1p1p1p/9/9/P1P1P1P1P/1C2N2C1/9/R1BAKAB1R b - - 12 37",
  "time_total_ms": 600000,
  "time_per_move_ms": 30000,
  "time_inc_ms": 0,
  "allow_spectate": true,
  "allow_undo": false,
  "undo_limit": 3,
  "moves_available": true,
  "started_at": "2026-09-23T09:40:00.000Z",
  "ended_at": "2026-09-23T09:58:22.410Z",
  "created_at": "2026-09-23T09:39:55.000Z",
  "moves": [
    {
      "seq": 1,
      "side": "red",
      "iccs": "h2e2",
      "notation": "炮二平五",
      "captured": null,
      "is_check": false,
      "clock_remaining_ms": 600000,
      "think_ms": 3200,
      "score_after": 15,
      "score_loss": 0,
      "level": "best"
    },
    {
      "seq": 2,
      "side": "black",
      "iccs": "h9g7",
      "notation": "马8进7",
      "captured": null,
      "is_check": false,
      "clock_remaining_ms": 597800,
      "think_ms": 5400,
      "score_after": -18,
      "score_loss": 33,
      "level": "good"
    }
  ],
  "move_stats": {
    "red":   { "best": 22, "good": 9, "dubious": 4, "blunder": 1, "missed": 0 },
    "black": { "best": 18, "good": 10, "dubious": 6, "blunder": 2, "missed": 1 },
    "turning_points": [
      { "ply": 51, "score_loss": 210, "side": "black", "note": "弃马抢攻失误" },
      { "ply": 67, "score_loss": 480, "side": "black", "note": "漏看绝杀" }
    ]
  }
}
```

**关键设计说明**

| 项 | 说明 |
|---|---|
| `moves_available` | **归档标志**。`true` = `game_moves` 行存在；`false` = 该局已超过 24 个月保留期，着法明细已归档删除（[09 §7.1](09-数据模型与存储设计.md)），此时 `moves` 为空数组，客户端应改从 `GET /games/{id}/export?format=iccs` 取 `iccs_text` 重放 |
| `include=fens` | 每着附带 `fen_after`。若数据库该列为 NULL，服务端**实时计算**（消费者未必写过）。响应体从 ~15 KiB 增至 ~26 KiB（160 着 × 58 B ≈ 9.3 KiB 增量）。**默认关闭** |
| 不内联讲解 | 讲解单独端点（§3.3.3）。理由：单局讲解约 152 条 × 500 B ≈ 76 KiB（[09 §10.2](09-数据模型与存储设计.md)），内联会让详情响应体膨胀 2 倍以上，而复盘 UI 通常先渲染棋盘再按需拉讲解 |
| `red` / `black` 对象 | 昵称与积分变动做**冗余内联**（快照字段），避免客户端为渲染一局详情再发 N 个用户查询 |
| 不返回 `id`（着法主键） | 着法用 `(game_id, seq)` 定位，`bigint` 主键不对外（§1.4） |

**错误码**：`GAME004`（对局不存在）、`GAME005`（无权查看该对局）、`SYS004`（`include` 取值非法）、`AUTH002`、`SYS001`

---

#### 3.3.3 对局讲解（逐着）

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /api/v1/games/{game_id}/coach-notes` |
| 鉴权 | **可选**（同 §3.3.2） |
| 幂等 | 是 |
| 限流 | 用户/IP 维度 **60 次/分钟** |
| 分页 | 无（用 `from_ply` / `to_ply` 区间代替；讲解是稠密序列，游标无意义） |

**查询参数**

| 参数 | 类型 | 必需 | 默认 | 约束 | 说明 |
|---|---|---|---|---|---|
| `from_ply` | integer | 否 | `1` | ≥ 1 | 起始着（含） |
| `to_ply` | integer | 否 | `from_ply + 59` | ≤ `from_ply + 199` | 结束着（含）。**单次最多 200 条**，防止一次拉全量 |
| `side` | string | 否 | — | `red` \| `black` | 只看某方 |

**请求示例**

```
GET /api/v1/games/018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f50/coach-notes?from_ply=1&to_ply=10
```

**响应示例**（`200 OK`）

```json
{
  "game_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f50",
  "from_ply": 1,
  "to_ply": 10,
  "notes_available": true,
  "notes": [
    {
      "ply": 1,
      "side": "red",
      "iccs": "h2e2",
      "notation": "炮二平五",
      "level": "best",
      "score_loss": 0,
      "tactics": [
        { "id": "central_control", "name": "中路控制", "category": "relation", "confidence": 0.82 }
      ],
      "headline": "红方炮二平五，最佳着法。",
      "detail": "中炮直取中路，是象棋最主流的开局体系之一。此时黑方需要及时出马保护中路，否则红方将形成持续压制。",
      "suggestion": null,
      "source": "llm",
      "llm_status": "enhanced"
    },
    {
      "ply": 2,
      "side": "black",
      "iccs": "h9g7",
      "notation": "马8进7",
      "level": "good",
      "score_loss": 33,
      "tactics": [],
      "headline": "黑方马8进7，不错。",
      "detail": "起马保卒，是应对中炮的常见应法，中路虽受压力但阵型完整。",
      "suggestion": null,
      "source": "local",
      "llm_status": "none"
    }
  ]
}
```

| 字段 | 说明 |
|---|---|
| `notes_available` | `false` = 该局讲解已过保留期归档（与 `moves_available` 同源条件，[09 §7.2](09-数据模型与存储设计.md)） |
| `iccs` / `notation` | 冗余带上着法标识，使客户端渲染讲解卡片**无需与 `moves` 数组做关联**（复盘 UI 只拉讲解时也能直接展示） |
| `source` / `llm_status` | 对应 UI 的"增强中 → 已增强"渐进状态（[05 §6.5](05-战法讲解引擎设计.md)）。`llm_status = "failed"` **不向用户展示失败**，保持本地讲解文本 |
| `tactics[].category` | `structure` / `relation` / `formation` / `opening`（[05 §7](05-战法讲解引擎设计.md)）。客户端对 `formation` 且 `confidence < 0.7` 的标签以弱样式展示（[05 §3.4](05-战法讲解引擎设计.md) 的待校准声明） |

**错误码**：`GAME004`、`GAME005`、`SYS004`（区间非法或跨度过大）、`SYS001`

---

#### 3.3.4 我的统计

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /api/v1/users/me/stats` |
| 鉴权 | **必需** |
| 幂等 | 是 |
| 限流 | 用户维度 **60 次/分钟** |

**查询参数**

| 参数 | 类型 | 必需 | 默认 | 说明 |
|---|---|---|---|---|
| `scope` | string | 否 | `all` | `all`（历史总览）\| `season`（当前赛季）\| `recent`（最近 20 局） |
| `season_id` | string | 否 | — | `scope=season` 且不传时使用当前赛季 |

**请求示例**

```
GET /api/v1/users/me/stats?scope=all
Authorization: Bearer eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...
```

**响应示例**（`200 OK`）

```json
{
  "scope": "all",
  "wins": 12,
  "losses": 5,
  "draws": 1,
  "total_games": 18,
  "win_rate": 0.667,
  "aborted_games": 0,
  "rating": 1524,
  "peak_rating": 1536,
  "current_streak": { "type": "win", "count": 3 },
  "side_stats": {
    "red":   { "wins": 8, "losses": 2, "draws": 1 },
    "black": { "wins": 4, "losses": 3, "draws": 0 }
  },
  "end_reason_stats": {
    "checkmate": 6,
    "resign": 5,
    "timeout": 3,
    "agreed_draw": 1,
    "disconnect": 1,
    "stalemate": 1,
    "sixty_move": 1,
    "insufficient_material": 0,
    "repetition": 0,
    "abort": 0
  },
  "move_quality": {
    "best": 402,
    "good": 175,
    "dubious": 88,
    "blunder": 21,
    "missed": 4
  },
  "recent_form": ["win", "win", "loss", "win", "draw"]
}
```

> `win_rate` 保留 3 位小数。`recent_form` 为最近 5 局结果（**不含**和棋以外的未完成局）。
> `move_quality` 来自 `games.move_stats` 的聚合，**归档后仍然可用**（[09 §3.4](09-数据模型与存储设计.md)）——这正是把汇总统计冗余到 `games` 行的价值。

**错误码**：`AUTH001`、`AUTH002`、`SYS004`、`SYS001`

---

### 3.4 大厅域

#### 3.4.1 进行中对局列表

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /api/v1/lobby/games` |
| 鉴权 | **公开**（FR-06.01 大厅浏览） |
| 幂等 | 是 |
| 限流 | IP 维度 **60 次/分钟** |
| 缓存 | 服务端内部可缓存 ≤ 3 秒（⚠️ 设计初始值）；响应带 `Cache-Control: public, max-age=2` |

**查询参数**

| 参数 | 类型 | 必需 | 默认 | 约束 | 说明 |
|---|---|---|---|---|---|
| `limit` | integer | 否 | `20` | 1..100 | 每页条数 |
| `offset` | integer | 否 | `0` | ≥ 0 | **偏移分页**（理由见 §5.2） |
| `sort` | string | 否 | `rating_desc` | `rating_desc` \| `started_desc` \| `spectators_desc` | 排序：按平均等级分 / 按开局时间 / 按观战人数（FR-06.01"按等级/时长排序"） |
| `mode` | string | 否 | — | `ranked` \| `room` \| `casual` | 按模式过滤 |
| `min_rating` | integer | 否 | — | 0..4000 | 只看平均等级分 ≥ 该值的对局（**学习场景**：看高手对局） |

**请求示例**

```
GET /api/v1/lobby/games?sort=rating_desc&limit=20&min_rating=1800
```

**响应示例**（`200 OK`）

```json
{
  "items": [
    {
      "game_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f60",
      "room_code": "317205",
      "mode": "ranked",
      "status": "ongoing",
      "red": { "nickname": "楚河大师", "rating": 1902, "is_ai": false },
      "black": { "nickname": "汉界棋手", "rating": 1887, "is_ai": false },
      "total_plies": 42,
      "red_time_left_ms": 381200,
      "black_time_left_ms": 402900,
      "spectator_count": 17,
      "spectator_limit": 50,
      "allow_spectate": true,
      "started_at": "2026-09-23T10:02:11.000Z",
      "elapsed_ms": 612300
    }
  ],
  "page": 1,
  "page_size": 20,
  "total": 143
}
```

| 字段 | 说明 |
|---|---|
| `red` / `black` 中**不含 `user_id`** | 大厅是公开列表，避免批量枚举用户 ID；进入观战后由 `GET /rooms/{code}` 或 WS 快照提供完整信息 |
| `red_time_left_ms` | 由 Redis 房间状态提供（**实时值**，[ADR-014](14-决策记录ADR.md#adr-014) 权威时钟）。若 Redis 中无该房间（跨实例未同步），字段为 `null` |
| `elapsed_ms` | 已进行时长，服务端计算（避免客户端时钟不可信） |
| `allow_spectate = false` 的对局**不出现在此列表中** | 与 FR-05.03"是否允许观战"一致；`spectator_limit` 已满的对局**仍出现**（但 `spectator_count == spectator_limit`），客户端应置灰"进入观战"按钮 |

**错误码**：`SYS004`、`SYS001`

---

#### 3.4.2 房间信息查询

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /api/v1/rooms/{room_code}` |
| 鉴权 | **公开** |
| 幂等 | 是 |
| 限流 | IP 维度 **120 次/分钟** |

**路径参数**

| 参数 | 类型 | 说明 |
|---|---|---|
| `room_code` | string | 6 位房间号（[09 §3.4](09-数据模型与存储设计.md) `games.room_code`） |

**请求示例**

```
GET /api/v1/rooms/317205
```

**响应示例（进行中）**（`200 OK`）

```json
{
  "room_code": "317205",
  "game_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f60",
  "status": "ongoing",
  "mode": "ranked",
  "red": { "nickname": "楚河大师", "rating": 1902, "is_ai": false },
  "black": { "nickname": "汉界棋手", "rating": 1887, "is_ai": false },
  "time_total_ms": 600000,
  "time_per_move_ms": 30000,
  "time_inc_ms": 0,
  "allow_spectate": true,
  "allow_undo": false,
  "undo_limit": 3,
  "spectator_count": 17,
  "spectator_limit": 50,
  "red_time_left_ms": 381200,
  "black_time_left_ms": 402900,
  "total_plies": 42,
  "started_at": "2026-09-23T10:02:11.000Z"
}
```

**响应示例（未开局 / 已结束房间）**（`200 OK`）

```json
{
  "room_code": "317205",
  "game_id": null,
  "status": "closed",
  "mode": null,
  "red": null,
  "black": null,
  "time_total_ms": null,
  "time_per_move_ms": null,
  "time_inc_ms": null,
  "allow_spectate": null,
  "allow_undo": null,
  "undo_limit": null,
  "spectator_count": 0,
  "spectator_limit": null,
  "red_time_left_ms": null,
  "black_time_left_ms": null,
  "total_plies": null,
  "started_at": null
}
```

> **"未开局房间"的处理**：房间创建后、开局前**不写 PostgreSQL**（[09 §1.2](09-数据模型与存储设计.md)：房间状态只在 Redis）。因此本端点对未开局房间会返回 `status = "closed"` 且 `game_id = null`。**这是刻意的取舍**——未开局房间是瞬时状态，不值得为其建 PG 记录；客户端应通过 WS 的 `RoomCreate` / `RoomJoin` 帧（[06-联网协议](06-联网对战与实时通信协议.md)）处理未开局房间。
>
> **房间创建与加入走 WS 而非 REST**：理由见 §7.2。
>
> **`spectator_count >= spectator_limit`** 时进入观战应被拒绝（`ROOM006`），但该拒绝发生在 WS 订阅时，本端点只报告计数。

**错误码**：`ROOM001`（房间不存在）、`ROOM007`（房间已关闭——**本端点不用此码**，因为"已关闭"通过 `status` 字段表达；`ROOM007` 供 WS 订阅时使用）、`SYS004`（房间号格式非法）、`SYS001`

---

### 3.5 棋谱域

#### 3.5.1 导出棋谱

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /api/v1/games/{game_id}/export` |
| 鉴权 | **可选**（同 §3.3.2 的可见性规则） |
| 幂等 | 是 |
| 限流 | 用户/IP 维度 **20 次/小时** |
| 响应类型 | `text/plain; charset=utf-8`（带 `Content-Disposition: attachment`） |

**查询参数**

| 参数 | 类型 | 必需 | 默认 | 约束 | 说明 |
|---|---|---|---|---|---|
| `format` | string | 否 | `chinese` | `iccs` \| `chinese` \| `pgn` | 导出格式 |
| `with_comments` | boolean | 否 | `false` | — | 是否附带讲解注释（**仅 `pgn` 与 `chinese` 支持**） |
| `with_clock` | boolean | 否 | `false` | — | 是否附带每着剩余时间 |

**三种格式定义**

| `format` | 内容 | 用途 |
|---|---|---|
| `iccs` | 空格分隔的 ICCS 着法串 + 起止 FEN 头 | 机器可读，供第三方工具互通（[ADR-013](14-决策记录ADR.md#adr-013)）；**归档后的唯一可用格式**（`moves_available = false` 时仍可导出） |
| `chinese` | 中文记谱（如 `1. 炮二平五 马8进7`），含回合编号 | 人类阅读、打印、分享（FR-09.05） |
| `pgn` | 类 PGN 结构：`[Event]` / `[Red]` / `[Black]` / `[Result]` 标签 + 着法序列 | 与棋类工具互操作。**注意**：中国象棋无标准 PGN 规范，本格式为"PGN 风格"（[01 §4.9](01-需求规格说明书.md) 表述为"PGN 风格"），字段定义见下 |

**请求示例**

```
GET /api/v1/games/018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f50/export?format=chinese&with_comments=true
Authorization: Bearer eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...
```

**响应示例（`format=chinese`）**

```text
2026-09-23 楚河大师 vs 汉界棋手 · 红方胜（将死）
时限：局时 10 分钟 / 步时 30 秒

 1. 炮二平五   马8进7
 2. 马二进三   车9平8
 3. 车一平二   马2进3
 ...
37. 车八平六   将5平4  （将死）

红方：优 22 / 良 9 / 疑 4 / 劣 1 / 漏 0
黑方：优 18 / 良 10 / 疑 6 / 劣 2 / 漏 1

【第 1 着 · 红方 · 炮二平五】最佳着法
中炮直取中路，是象棋最主流的开局体系之一。此时黑方需要及时出马保护中路。
```

**响应示例（`format=iccs`）**

```text
[InitialFEN "rnbakabnr/9/1c5c1/p1p1p1p1p/9/9/P1P1P1P1P/1C5C1/9/RNBAKABNR w - - 0 1"]
[FinalFEN "r1bakabnr/9/1cn4c1/p1p1p1p1p/9/9/P1P1P1P1P/1C2N2C1/9/R1BAKAB1R b - - 12 37"]
[Result "1-0"]
[RulesVersion "2026.1"]

h2e2 h9g7 b0c2 b9c7 h0g2 i9h9 ... i0f0 e9d9
```

**响应示例（`format=pgn`）**

```text
[Event "弈道 天梯对局"]
[Site "xq-server"]
[Date "2026.09.23"]
[Round "-"]
[Red "楚河大师"]
[Black "汉界棋手"]
[RedElo "1512"]
[BlackElo "1508"]
[Result "1-0"]
[Termination "checkmate"]
[TimeControl "600+30"]
[InitialFEN "rnbakabnr/9/1c5c1/p1p1p1p1p/9/9/P1P1P1P1P/1C5C1/9/RNBAKABNR w - - 0 1"]

1. h2e2 h9g7 2. b0c2 b9c7 3. h0g2 i9h9 ...
37. i0f0 e9d9 1-0
```

| 格式约定 | 说明 |
|---|---|
| `Result` | `1-0`（红胜）/ `0-1`（黑胜）/ `1/2-1/2`（和）/ `*`（未结束） |
| 字符编码 | UTF-8。PGN 风格输出**不加 BOM**（避免解析器把 BOM 当标签首字符） |
| `Content-Disposition` | `attachment; filename="xq_018f2a3c_chinese.txt"`（文件名用对局 ID 前 8 位 + 格式，避免中文文件名在不同平台的编码问题） |
| 归档对局的导出 | `moves_available = false` 时**仍可导出**：`iccs` 直接读 `games.iccs_text`；`chinese` 与 `pgn` 需把 ICCS 串反解为中文记谱，服务端用 `xq-core::notation` 从 `initial_fen` 重放生成。**`with_comments` 对归档对局无效**（讲解已删除，返回不含注释的文本） |
| **明确不做** | 导出图片（[01 §4.9 FR-09.05](01-需求规格说明书.md) 列了"图片"格式）——服务端无图形渲染能力，属客户端职责。**本文不定义图片导出端点** |

**错误码**：`GAME004`、`GAME005`、`GAME007`（棋谱格式不支持）、`GAME006`（对局尚未结束，无法导出完整棋谱——仅当 `status = ongoing` 时）、`SYS004`、`RATE001`、`SYS001`

---

#### 3.5.2 导入棋谱

| 项 | 值 |
|---|---|
| 方法 / 路径 | `POST /api/v1/games/import` |
| 鉴权 | **必需** |
| 幂等 | **是**（纯解析、无副作用；见 §8.2） |
| 限流 | 用户维度 **30 次/小时** |
| 请求体大小 | ≤ 64 KiB |

**设计决定：导入不创建对局记录，只返回解析结果。**

理由：
1. **导入的真实用途**是（a）复盘别人的棋谱、（b）从任意局面开始人机对战/分支分析（FR-09.06）、（c）残局挑战（FR-01.14）。三者都是**客户端本地场景**，不需要服务端存一条 `games` 记录。
2. 若允许导入落库，会带来"谁能删这条记录"、"是否计入战绩"、"是否参与排行榜"三类歧义，且给了用户**直接构造 `games` 行**的入口，污染数据可信度。
3. 解析（含逐着合法性校验）恰恰是服务端的强项——用权威 `xq-core` 校验，防止客户端把非法棋谱当作合法局面加载。

**请求参数**

| 字段 | 类型 | 必需 | 约束 | 说明 |
|---|---|---|---|---|
| `format` | string | 是 | `iccs` \| `chinese` \| `pgn` | 输入格式 |
| `content` | string | 是 | ≤ 64 KiB | 棋谱文本 |
| `initial_fen` | string | 否 | 合法 FEN | **仅当输入棋谱不含起始局面标签时**使用；缺省为标准初始局面 |
| `validate_legality` | boolean | 否 | 默认 `true` | 是否用 `xq-core` 逐着校验合法性。设为 `false` 时仅解析不着法（**用于导入"教学演示用"的特殊局面**，此时响应中 `illegal_at_ply` 会标出首个非法着） |

**请求示例**

```json
POST /api/v1/games/import
Authorization: Bearer eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...
Content-Type: application/json
Idempotency-Key: 018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f31

{
  "format": "chinese",
  "content": "1. 炮二平五 马8进7\n2. 马二进三 车9平8",
  "validation_legality": true
}
```

**响应示例**（`200 OK`）

```json
{
  "initial_fen": "rnbakabnr/9/1c5c1/p1p1p1p1p/9/9/P1P1P1P1P/1C5C1/9/RNBAKABNR w - - 0 1",
  "final_fen": "rnbakabnr/9/1c2c4/p1p1p1p1p/9/9/P1P1P1P1P/1C2N4/9/RNBAKAB1R b - - 2 3",
  "total_plies": 4,
  "legal": true,
  "illegal_at_ply": null,
  "rules_version": "2026.1",
  "moves": [
    { "seq": 1, "side": "red",   "iccs": "h2e2", "notation": "炮二平五" },
    { "seq": 2, "side": "black", "iccs": "h9g7", "notation": "马8进7" },
    { "seq": 3, "side": "red",   "iccs": "h0g2", "notation": "马二进三" },
    { "seq": 4, "side": "black", "iccs": "i9h9", "notation": "车9平8" }
  ]
}
```

**响应示例（含非法着法）**（`200 OK`）

```json
{
  "initial_fen": "rnbakabnr/9/1c5c1/p1p1p1p1p/9/9/P1P1P1P1P/1C5C1/9/RNBAKABNR w - - 0 1",
  "final_fen": "rnbakabnr/9/1c2c4/p1p1p1p1p/9/9/P1P1P1P1P/1C2N4/9/RNBAKAB1R b - - 2 3",
  "total_plies": 5,
  "legal": false,
  "illegal_at_ply": 5,
  "illegal_reason": "GAME001",
  "rules_version": "2026.1",
  "moves": [
    { "seq": 1, "side": "red",   "iccs": "h2e2", "notation": "炮二平五" },
    { "seq": 2, "side": "black", "iccs": "h9g7", "notation": "马8进7" },
    { "seq": 3, "side": "red",   "iccs": "h0g2", "notation": "马二进三" },
    { "seq": 4, "side": "black", "iccs": "i9h9", "notation": "车9平8" },
    { "seq": 5, "side": "red",   "iccs": "e2e9", "notation": "炮五进七" }
  ]
}
```

> **非法棋谱返回 `200` 而非 `4xx`**：解析**成功**了，只是内容不合规——这是"业务结果"而非"请求错误"。客户端据 `legal: false` + `illegal_at_ply` 高亮问题着法（对复盘场景极有用："这份棋谱第 5 着有误"）。
> **什么时候返回 `422 GAME008`**：`content` **无法解析**（语法错误，如 ICCS 串长度不对、中文记谱棋子名非法）→ `GAME008`。

**错误码**：`AUTH001`、`AUTH002`、`GAME007`（`format` 不支持）、`GAME008`（棋谱解析失败）、`SYS004`（`content` 超长或 `initial_fen` 非法）、`SYS003`（请求体过大）、`RATE001`

---

### 3.6 匹配域 `/api/v1/match`

> **通道说明**：匹配**指令**走 REST（可靠、可重试、幂等、断线后仍可查询当前状态），匹配**结果推送**走 WS（`MatchFound` 帧）。理由见 §7.2。

#### 3.6.1 开始匹配

| 项 | 值 |
|---|---|
| 方法 / 路径 | `POST /api/v1/match` |
| 鉴权 | **必需** |
| 幂等 | **是**（同一用户重复提交返回首次结果；见 §8.3） |
| 限流 | 用户维度 **10 次/分钟** |

**请求参数**

| 字段 | 类型 | 必需 | 默认 | 约束 | 说明 |
|---|---|---|---|---|---|
| `mode` | string | 否 | `ranked` | 仅 `ranked` | 本期只支持天梯匹配；`casual` / `room` 走房间流程 |
| `time_total_ms` | integer | 否 | `600000` | 60,000..3,600,000 | 偏好的局时（10 分钟） |
| `time_per_move_ms` | integer | 否 | `30000` | 5,000..300,000 | 偏好的步时（30 秒） |
| `prefer_side` | string | 否 | `random` | `random` \| `red` \| `black` | 执子偏好；`random` 时不参与筛选 |

**请求示例**

```json
POST /api/v1/match
Authorization: Bearer eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...
Content-Type: application/json
Idempotency-Key: 018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f32

{
  "mode": "ranked",
  "time_total_ms": 600000,
  "time_per_move_ms": 30000,
  "prefer_side": "random"
}
```

**响应示例**（`202 Accepted`）

```json
{
  "ticket_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f70",
  "status": "queued",
  "mode": "ranked",
  "rating_at_enqueue": 1524,
  "band_min": 1324,
  "band_max": 1724,
  "enqueued_at": "2026-09-23T10:30:00.000Z",
  "estimated_wait_ms": 12000,
  "queue_length": 87
}
```

| 字段 | 说明 |
|---|---|
| `202 Accepted` | 匹配是**异步**过程：请求已被接受，结果通过 WS `MatchFound` 帧推送 |
| `ticket_id` | 本次排队票据（UUID v7），用于取消与状态查询 |
| `band_min` / `band_max` | **初始**可接受分差区间（`1524 ± 200`，[09 §3.8](09-数据模型与存储设计.md)）。该区间随等待时长**逐步放宽**（FR-07.02） |
| `estimated_wait_ms` | 预估等待时长。⚠️ **无实测依据**，来源是"当前队列长度 × 历史平均匹配耗时"的粗估（服务端从 `match_queue_logs` 统计）。**客户端不应把它当承诺**，UI 上应表述为"预计" |
| `queue_length` | 当前队列人数，供 UI 提示"棋友正在赶来" |

**错误码**：`AUTH001`、`AUTH002`、`MATCH001`（已在匹配队列中）、`MATCH004`（天梯模式被限制——封禁或反作弊标记）、`MATCH003`（匹配服务暂不可用）、`SYS004`、`RATE001`

---

#### 3.6.2 取消匹配

| 项 | 值 |
|---|---|
| 方法 / 路径 | `DELETE /api/v1/match` |
| 鉴权 | **必需** |
| 幂等 | **是**（不在队列中也返回 `204`） |
| 限流 | 用户维度 **10 次/分钟** |

**查询参数**

| 参数 | 类型 | 必需 | 说明 |
|---|---|---|---|
| `ticket_id` | string | 否 | 指定取消哪张票据；缺省时取消该用户当前全部排队票据 |

**请求示例**

```
DELETE /api/v1/match?ticket_id=018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f70
Authorization: Bearer eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...
```

**响应示例**：`204 No Content`

> 服务端动作：`ZREM match:queue {user_id}` + `DEL match:queue:member:{user_id}` + 写一条 `match_queue_logs`（`outcome = 'cancelled'`）。
> 若此刻已匹配成功（`MatchFound` 已推送、对局已创建），则**取消失败**，返回 `409 MATCH002` 并附带 `game_id`，客户端应跳转到对局而非停留在匹配界面。

**错误码**：`AUTH001`、`AUTH002`、`MATCH002`（不在匹配队列中——含已被匹配的情况）、`SYS004`、`RATE001`

---

#### 3.6.3 匹配状态查询

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /api/v1/match/status` |
| 鉴权 | **必需** |
| 幂等 | 是 |
| 限流 | 用户维度 **60 次/分钟** |

**请求参数**：无

**请求示例**

```
GET /api/v1/match/status
Authorization: Bearer eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...
```

**响应示例（排队中）**（`200 OK`）

```json
{
  "status": "queued",
  "ticket_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f70",
  "mode": "ranked",
  "rating_at_enqueue": 1524,
  "band_min": 1224,
  "band_max": 1824,
  "enqueued_at": "2026-09-23T10:30:00.000Z",
  "waited_ms": 34500,
  "queue_length": 76,
  "game_id": null,
  "room_code": null
}
```

**响应示例（已匹配）**（`200 OK`）

```json
{
  "status": "matched",
  "ticket_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f70",
  "mode": "ranked",
  "rating_at_enqueue": 1524,
  "band_min": 1424,
  "band_max": 1624,
  "enqueued_at": "2026-09-23T10:30:00.000Z",
  "waited_ms": 8200,
  "queue_length": 76,
  "game_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f60",
  "room_code": "317205"
}
```

**响应示例（未排队）**（`200 OK`）

```json
{
  "status": "idle",
  "ticket_id": null,
  "mode": null,
  "rating_at_enqueue": null,
  "band_min": null,
  "band_max": null,
  "enqueued_at": null,
  "waited_ms": 0,
  "queue_length": 76,
  "game_id": null,
  "room_code": null
}
```

| `status` | 含义 | 客户端行为 |
|---|---|---|
| `idle` | 未在队列中 | 展示"开始匹配"入口 |
| `queued` | 排队中 | 展示等待动画与 `waited_ms`、`band_*`（体现"范围正在放宽"） |
| `matched` | 已匹配，对局已创建 | 跳转到 `game_id` / `room_code`，同时等待 WS `MatchFound` 帧（两条路径都能到达，**REST 是兜底**——若 WS 推送丢失，轮询本端点仍能发现已匹配） |
| `timeout` | 超时放弃（FR-07.08，60 秒未匹配） | 提示"暂未找到对手"，提供"继续等待"或"转人机对战"选项 |

**错误码**：`AUTH001`、`AUTH002`、`MATCH003`、`SYS001`

---

### 3.7 排行榜域

#### 3.7.1 排行榜（总榜 / 周榜）

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /api/v1/leaderboard` |
| 鉴权 | **公开** |
| 幂等 | 是 |
| 限流 | IP 维度 **60 次/分钟** |
| 分页 | **偏移分页**（§5.2） |
| 缓存 | 服务端内部缓存 ≤ 60 秒 ⚠️ 设计初始值 |

**查询参数**

| 参数 | 类型 | 必需 | 默认 | 约束 | 说明 |
|---|---|---|---|---|---|
| `board` | string | 否 | `all` | `all` \| `weekly` | 总榜 / 周榜 |
| `page` | integer | 否 | `1` | 1..`total_pages` | 页码（**从 1 开始**） |
| `page_size` | integer | 否 | `50` | 1..200 | 每页条数 |
| `season_id` | string | 否 | — | UUID v7 | `board=weekly` 时指定赛季；缺省为当前赛季 |
| `week` | string | 否 | — | `YYYY-Www`（ISO 8601 周） | `board=weekly` 时指定周；缺省为当前周 |

**请求示例**

```
GET /api/v1/leaderboard?board=weekly&page=1&page_size=50
```

**响应示例**（`200 OK`）

```json
{
  "board": "weekly",
  "season_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f40",
  "season_name": "2026-S4",
  "week": "2026-W39",
  "week_starts_at": "2026-09-21T00:00:00.000Z",
  "week_ends_at": "2026-09-27T23:59:59.999Z",
  "metric": "weekly_rating_gain",
  "items": [
    {
      "rank": 1,
      "user_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f23",
      "nickname": "楚河大师",
      "avatar_id": "preset_07",
      "rating": 1902,
      "week_gain": 186,
      "week_games": 24,
      "week_wins": 19,
      "week_losses": 4,
      "week_draws": 1
    },
    {
      "rank": 2,
      "user_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f24",
      "nickname": "汉界棋手",
      "avatar_id": "preset_03",
      "rating": 1887,
      "week_gain": 154,
      "week_games": 20,
      "week_wins": 15,
      "week_losses": 4,
      "week_draws": 1
    }
  ],
  "page": 1,
  "page_size": 50,
  "total": 12876,
  "total_pages": 258,
  "generated_at": "2026-09-23T10:30:00.000Z",
  "my_rank": null
}
```

| 字段 | 说明 |
|---|---|
| `metric` | 榜的排序依据。`all` 时为 `rating`；`weekly` 时为 `weekly_rating_gain`（**本周净增分**，非绝对分——这是"周榜"的正确语义，避免高分玩家永远霸榜） |
| `total` / `total_pages` | 由 Redis Sorted Set 基数得出（`ZCARD`），**成本 O(1)**。这正是选偏移分页的前提（§5.2） |
| `generated_at` | 榜单缓存生成时间。客户端应展示"数据更新于 X 分钟前"，避免用户误解为实时 |
| `my_rank` | 携带鉴权时返回**我自己**的名次与摘要（便于 UI 把"我的排名"固定置顶）。未鉴权为 `null` |
| `week_starts_at` / `week_ends_at` | 周榜的统计窗口边界（ISO 8601 周，**周一 00:00 UTC 起** ⚠️ 是否改用北京时间需产品确认）。客户端据此展示"距离本周结束还有 X 小时" |
| 无 `my_rank` 时的默认行为 | 未登录用户看不到自己的名次。**这是刻意的**：周榜需要用户身份，匿名访问只能看公开部分 |

**错误码**：`SYS004`（`page` / `page_size` / `week` 非法）、`SYS006`（`season_id` 不存在）、`SYS001`

---

#### 3.7.2 当前赛季信息

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /api/v1/seasons/current` |
| 鉴权 | **公开** |
| 幂等 | 是 |
| 限流 | IP 维度 **60 次/分钟** |

**请求参数**：无

**请求示例**

```
GET /api/v1/seasons/current
```

**响应示例**（`200 OK`）

```json
{
  "id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f40",
  "seq": 4,
  "name": "2026-S4",
  "status": "active",
  "starts_at": "2026-09-01T00:00:00.000Z",
  "ends_at": "2026-11-30T23:59:59.999Z",
  "reset_mode": "soft",
  "remaining_ms": 5889599999
}
```

> **存在的理由**：周榜的周次边界、赛季重置时间（FR-07.06）、`user_season_stats` 的归属都需要客户端知道"当前赛季是哪个、什么时候结束"。没有这个端点，客户端只能靠猜。
> **`config` 字段不返回**：赛季参数（ELO K 值、段位分界线）属服务端内部实现，客户端不应据此做任何计算。`reset_mode` 是例外——客户端需要用它解释"为什么我的分数变了"。

**错误码**：`SYS006`（无 active 赛季——**这是异常状态**，服务端应保证始终有一个 active 赛季，见 [09 §3.3](09-数据模型与存储设计.md) 的部分唯一索引）、`SYS001`

---

### 3.8 举报域

#### 3.8.1 提交举报

| 项 | 值 |
|---|---|
| 方法 / 路径 | `POST /api/v1/reports` |
| 鉴权 | **必需**（举报必须实名，防止滥用） |
| 幂等 | **是**（同一人对同一局同一理由只能提交一次，由 [09 §3.9](09-数据模型与存储设计.md) 的部分唯一索引 `reports_dedup_idx` 保证） |
| 限流 | 用户维度 **5 次/小时** |
| 请求体大小 | ≤ 16 KiB |

**请求参数**

| 字段 | 类型 | 必需 | 约束 | 说明 |
|---|---|---|---|---|
| `reason` | string | 是 | `cheating` \| `abuse` \| `flood` \| `impersonation` \| `other` | 举报理由 |
| `target_user_id` | string | 否 | UUID v7 | 被举报人 |
| `target_game_id` | string | 否 | UUID v7 | 关联对局（**最推荐的证据入口**） |
| `room_code` | string | 否 | 6 位 | 关联房间 |
| `detail` | string | 是 | 1..1000 | 举报描述 |
| `evidence` | object | 否 | — | 结构化证据，见下 |

**三项定位字段（`target_user_id` / `target_game_id` / `room_code`）至少提供一个**，否则 `400 SYS004`（与 [09 §3.9](09-数据模型与存储设计.md) 的 CHECK 约束一致）。

**`evidence` 允许的字段**（白名单，**多余字段一律忽略**）

| 字段 | 类型 | 说明 |
|---|---|---|
| `ply_range` | [integer, integer] | 可疑着法区间（如 `[40, 52]`） |
| `suspected_engine` | boolean | 是否怀疑使用引擎 |
| `notes` | string | 附加说明（≤ 500 字符） |
| `client_log_ref` | string | 客户端本地日志的引用标识（**不接收日志内容本身**，避免上传不明文件） |

> **`evidence` 为什么不复制棋谱**：棋谱用 `target_game_id` **引用**即可。复制会引入"证据与源数据不一致"的风险（源数据被归档后证据仍在），且给用户一个"通过举报上传任意大 JSON"的通道。服务端在处理工单时按 `target_game_id` 现场提取着法速度序列。

**请求示例**

```json
POST /api/v1/reports
Authorization: Bearer eyJhbGciOiJFZERTQSIsInR5cCI6IkpXVCJ9...
Content-Type: application/json
Idempotency-Key: 018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f33

{
  "reason": "cheating",
  "target_user_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f24",
  "target_game_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f60",
  "detail": "对方在中局阶段的着法速度与走法质量明显异常，疑似使用引擎辅助。",
  "evidence": {
    "ply_range": [40, 52],
    "suspected_engine": true,
    "notes": "连续 12 着平均思考时间不足 0.4 秒，且均为引擎首选。"
  }
}
```

**响应示例**（`201 Created`）

```json
{
  "id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f80",
  "status": "pending",
  "reason": "cheating",
  "target_user_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f24",
  "target_game_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f60",
  "created_at": "2026-09-23T10:35:00.000Z",
  "message": "举报已提交，我们会在 3 个工作日内处理。"
}
```

**重复提交的响应**（`200 OK`，`Idempotency-Key` 或唯一索引命中）

```json
{
  "id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f80",
  "status": "pending",
  "reason": "cheating",
  "target_user_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f24",
  "target_game_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f60",
  "created_at": "2026-09-23T10:35:00.000Z",
  "message": "你已提交过相同举报，无需重复提交。"
}
```

> **幂等语义**：重复提交**不报错**（返回 `200` + 原工单），因为"我提交了两次举报"是正常用户行为（网络重试/不确定是否成功），报错会造成困惑。
> `message` 中的"3 个工作日"⚠️ 是**服务承诺的口径，需与运营确认后固化**。

**错误码**：`AUTH001`、`AUTH002`、`USER001`（`target_user_id` 不存在）、`GAME004`（`target_game_id` 不存在）、`SYS004`（未提供任何定位字段 / `detail` 超长 / `evidence` 结构非法）、`SYS003`、`RATE001`

---

### 3.9 运维域

#### 3.9.1 存活检查 `/healthz`

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /healthz` |
| **不在一 `/api/v1` 下** | 运维端点不参与 API 版本演进 |
| 鉴权 | 公开（**应由反向代理限制为仅内网/仅探针访问**） |
| 幂等 | 是 |
| 限流 | **不限流** |

**请求参数**：无

**请求示例**

```
GET /healthz
```

**响应示例**（`200 OK`，**无依赖检查**）

```json
{
  "status": "ok",
  "service": "xq-server",
  "version": "0.1.0",
  "uptime_ms": 86412345
}
```

| 语义 | 说明 |
|---|---|
| **只报告进程存活** | 只要 HTTP 服务器能响应就返回 `200`。**不检查** PostgreSQL / Redis —— 这是 Liveness 探针的正确语义。若把依赖检查放进 `/healthz`，DB 短暂抖动会导致编排系统**误杀健康进程**（进而引发雪崩） |

**错误码**：无（永远返回 `200`，除非进程已死）

---

#### 3.9.2 就绪检查 `/readyz`

| 项 | 值 |
|---|---|
| 方法 / 路径 | `GET /readyz` |
| **不在 `/api/v1` 下** | 同 `/healthz` |
| 鉴权 | 公开（同 `/healthz`，应限制为内网） |
| 幂等 | 是 |
| 限流 | **不限流** |

**查询参数**

| 参数 | 类型 | 必需 | 默认 | 说明 |
|---|---|---|---|---|
| `verbose` | boolean | 否 | `false` | `true` 时返回每个依赖的详细检查结果 |

**请求示例**

```
GET /readyz?verbose=true
```

**响应示例（就绪）**（`200 OK`）

```json
{
  "status": "ready",
  "checks": {
    "postgres": { "status": "ok", "latency_ms": 1.8, "pool_size": 20, "pool_idle": 14 },
    "redis": { "status": "ok", "latency_ms": 0.4 },
    "persist_consumer": { "status": "ok", "active_streams": 143, "pending_total": 0 },
    "llm": { "status": "degraded", "latency_ms": null, "note": "未配置或不可用，讲解将降级为本地模板" }
  },
  "uptime_ms": 86412345
}
```

**响应示例（未就绪）**（`503 Service Unavailable`）

```json
{
  "status": "not_ready",
  "checks": {
    "postgres": { "status": "error", "latency_ms": null, "note": "连接池获取超时（3s）" },
    "redis": { "status": "ok", "latency_ms": 0.5 },
    "persist_consumer": { "status": "error", "active_streams": 143, "pending_total": 12840 },
    "llm": { "status": "disabled" }
  },
  "uptime_ms": 86412345
}
```

| 检查项 | 判定"未就绪"的条件 | 是否阻塞就绪 |
|---|---|---|
| `postgres` | 连接池取连接失败或超时 3 s | **是**（无 DB 无法提供任何业务端点） |
| `redis` | `PING` 失败或超时 1 s | **是**（无 Redis 无法提供房间/会话/限流） |
| `persist_consumer` | `pending_total > 1000` 或活跃 Stream 数连续 3 次检查为 0（异常） | **否**（降级：落库延迟上升，对局不受影响） |
| `llm` | 未配置 / 探测失败 | **否**（[ADR-004](14-决策记录ADR.md#adr-004) 明确降级为本地模板） |

| 语义 | 说明 |
|---|---|
| **就绪 = 可以接收流量** | `/readyz` 失败应从负载均衡摘除，但**不重启进程**（重启解决不了 DB 不可用） |
| **降级项不阻塞就绪** | `llm` 与 `persist_consumer` 都设计了降级路径，把它们做成硬依赖会违反"LLM 不得成为阻塞依赖"（[01 §10](01-需求规格说明书.md) 约束） |
| `verbose=false` 时的响应 | 仅 `{"status":"ready","uptime_ms":...}`，减少探针流量 |

**错误码**：无（用 `200` / `503` 表达两种状态，不用错误码结构——探针不应依赖业务错误码解析）

---

#### 3.9.3 关于 `/metrics`

> **本期不定义** `/metrics` REST 端点。[02 §9](02-系统架构设计.md) 要求"指标暴露"，但**指标接口不应经业务 API 服务器暴露**（会与业务流量争抢连接、且需要额外的鉴权面）。**约定**：由独立的指标导出器（或 sidecar）从进程内的指标注册表拉取，监听独立端口（⚠️ 端口与实现方式由 M4 部署阶段确定）。

---

## 4. 错误码全表

### 4.1 统一错误响应体

**错误码表是 REST 与 WS 的共享契约**：同一个 `ApiError` 枚举（定义在 `xq-protocol`）既作为 REST 的响应体，也作为 WS `ErrorFrame` 的错误载荷（[02 §10](02-系统架构设计.md)"对外边界（HTTP/WS）统一映射为 `ApiError` / `ErrorFrame`"）。因此下表中标注为「WS」的码在 REST 端点中**不会出现**，但仍是完整契约的一部分，客户端必须实现。

```json
{
  "error": {
    "code": "GAME001",
    "message": "该着法不合法",
    "detail": { "from": "h2", "to": "e5", "reason": "马腿被己方棋子阻挡" },
    "fields": null,
    "request_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f90",
    "retryable": false
  }
}
```

| 字段 | 类型 | 必需 | 说明 |
|---|---|---|---|
| `code` | string | **是** | 错误码，格式 `{域}{三位序号}`（[02 §10](02-系统架构设计.md)）。域为大写字母 3~5 位 |
| `message` | string | **是** | 面向用户的中文提示。**永不包含**堆栈、SQL 语句、文件路径、内部模块名（[02 §10](02-系统架构设计.md) 原则 1）。长度 1..500 |
| `detail` | object | 否 | 机器可读的结构化补充，各码自定义（如 `GAME001` 给出 `from` / `to` / `reason`）。缺省为 `null` |
| `fields` | array | 否 | 字段级校验错误。元素为 `{field, code, message}`。仅用于含字段校验的码（`SYS004` / `AUTH003` / `USER004` / `ROOM004`）。缺省为 `null` |
| `request_id` | string | **是** | 与响应头 `X-Request-Id` 一致。用户报障时凭此值可在服务端日志中定位完整上下文（[02 §10](02-系统架构设计.md) 原则 2） |
| `retryable` | boolean | **是** | **客户端可直接据此刻画"是否自动重试"**，避免在客户端硬编码整张码表。`true` 的码见 §4.4 |

**JSON Schema（信息性，供实现与测试校验）**

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "ApiErrorEnvelope",
  "type": "object",
  "required": ["error"],
  "additionalProperties": false,
  "properties": {
    "error": {
      "type": "object",
      "required": ["code", "message", "request_id", "retryable"],
      "additionalProperties": false,
      "properties": {
        "code":       { "type": "string", "pattern": "^[A-Z]{3,5}[0-9]{3}$" },
        "message":    { "type": "string", "minLength": 1, "maxLength": 500 },
        "detail":     { "type": ["object", "null"] },
        "fields": {
          "type": ["array", "null"],
          "items": {
            "type": "object",
            "required": ["field", "code", "message"],
            "additionalProperties": false,
            "properties": {
              "field":   { "type": "string", "minLength": 1 },
              "code":    { "type": "string", "pattern": "^[A-Z]{3,5}[0-9]{3}$" },
              "message": { "type": "string", "minLength": 1 }
            }
          }
        },
        "request_id": { "type": "string", "minLength": 1, "maxLength": 64 },
        "retryable":  { "type": "boolean" }
      }
    }
  }
}
```

**三条硬性原则**（沿用并强化 [02 §10](02-系统架构设计.md)）

| # | 原则 | 落实方式 |
|---|---|---|
| 1 | 对外**永不**包含内部细节 | `message` 由 `xq-protocol` 中的常量表提供，不拼接任何 `anyhow::Error` 的 `Display` 输出；CI 加测试断言错误响应的 JSON 中不含 `SELECT` / `.rs:` / `\` 等特征串 |
| 2 | 服务端**必须**保留完整上下文 | 每次错误响应生成时写一条 `tracing::error!`，携带 `request_id` / `user_id` / 端到端 `Span`（[02 §9](02-系统架构设计.md)） |
| 3 | 客户端遇**未知码**优雅降级 | 客户端只对已知码做特殊处理，其余统一走"通用错误提示 + `retryable` 判定的重试策略"，**不崩溃、不白屏**（[02 §10](02-系统架构设计.md) 原则 3） |

### 4.2 错误码表（46 个）

> **HTTP 语义约定**：`401` = 身份不可信（需重新认证）；`403` = 身份可信但无权；`404` = 资源不存在（**不区分"不存在"与"无权限"时统一用 404**，避免通过状态码枚举资源）；`409` = 状态冲突；`422` = 语义不合法（格式对但内容错）；`429` = 限流。

#### 4.2.1 `AUTH` —— 鉴权与账号（10 个）

| 错误码 | HTTP | 含义 | 客户端应如何处理 |
|---|---|---|---|
| `AUTH001` | 401 | 未认证：缺少 `Authorization` 头，或 `typ ≠ access`，或 token 格式非法 | ① 若本地存在 refresh token → **尝试静默刷新一次**后重放原请求；② 刷新失败或无 refresh token → 清凭据、跳登录页；③ **禁止**无限重试 |
| `AUTH002` | 401 | access token 已过期 | 用 refresh token 刷新（遵守 §2.3 的**单飞**约束），成功后重放原请求；刷新失败转 `AUTH005` 处理 |
| `AUTH003` | 400 | 密码强度不足 | 读取 `error.fields` 就地标红并展示**每一条**具体原因（长度/字符类型/弱密码/与用户名相似）；**不清空**用户已输入的内容 |
| `AUTH004` | 401 | 用户名或密码错误（**含改密时的原密码错误**） | 统一文案"用户名或密码错误"，**不提示"用户不存在"**；登录场景累加本地失败计数并在 3 次后展示"忘记密码"入口 |
| `AUTH005` | 401 | refresh token 无效或已失效（已被轮换 / 复用检测命中 / 撤销） | ① 立即清除本地凭据；② 跳登录页并提示"登录状态已失效，请重新登录"；③ **禁用自动重试**（重试只会再次触发复用检测） |
| `AUTH006` | 403 | 账号已被停用 | 展示封禁提示与申诉入口；清除本地凭据；**不自动重试** |
| `AUTH007` | 403 | 账号已注销 | 清除本地凭据，跳登录页 |
| `AUTH008` | 409 | 用户名已存在 | 注册表单就地标红 `username`，提示"该用户名已被使用"；**保留**用户填写的其他字段 |
| `AUTH009` | 429 | 登录尝试过于频繁（账号被临时锁定） | 读取 `error.detail.remaining_lock_ms`，禁用登录按钮并倒计时；**不轮询重试** |
| `AUTH010` | 403 | 无权限访问该资源（`role` 不足） | 展示"无权访问"；不重试 |

#### 4.2.2 `USER` —— 资料（4 个）

| 错误码 | HTTP | 含义 | 客户端应如何处理 |
|---|---|---|---|
| `USER001` | 404 | 用户不存在（或已硬删） | 展示"该用户不存在或已注销"；若来自列表跳转，返回列表 |
| `USER002` | 409 | 昵称已被占用（大小写不敏感唯一） | 表单就地标红 `nickname`，提示换一个 |
| `USER003` | 400 | 头像标识无效 | 回退到当前头像；重新拉取内置头像列表后让用户重选 |
| `USER004` | 400 | 昵称不符合规范 | 读取 `error.fields` 就地提示具体原因（长度/控制字符/前后空白） |

#### 4.2.3 `GAME` —— 对局与棋谱（10 个）

| 错误码 | HTTP | 含义 | 客户端应如何处理 |
|---|---|---|---|
| `GAME001` | 422 | 非法着法 | **REST**：出现在 `POST /games/import` 的解析结果中（作为 `illegal_reason`），客户端高亮第 `illegal_at_ply` 着。**WS**：由 `MoveRejected` 携带，客户端**回滚乐观更新** → 请求全量快照 → 提示原因（[ADR-003](14-决策记录ADR.md#adr-003)：回滚时播放"棋子退回"动画而非瞬间跳变） |
| `GAME002` | 409 | 未轮到该方走子（**WS**） | WS 场景：丢弃本地乐观更新并回滚。REST 层当前无承载该错误的端点 |
| `GAME003` | 409 | 对局已结束 | 停止一切对局操作；清除本地对局状态；跳转复盘页（FR-09） |
| `GAME004` | 404 | 对局不存在 | 展示"对局不存在"；退出到列表 |
| `GAME005` | 403 | 无权查看该对局（非参与方且不可观战） | 展示"该对局不公开"；返回上一级 |
| `GAME006` | 409 | 对局尚未结束，无法导出完整棋谱 | 提示"对局结束后可导出完整棋谱"；提供替代入口"导出当前局面"（**由客户端本地生成**，非服务端能力） |
| `GAME007` | 400 | 棋谱格式不支持 | 回退到默认格式（导出 `chinese` / 导入 `iccs`）并提示支持的格式列表 |
| `GAME008` | 422 | 棋谱解析失败（语法级） | 读取 `error.detail.offset` 与 `error.detail.expect`，在导入文本中定位错误位置；**保留**用户输入 |
| `GAME009` | 409 | 悔棋请求不可用或超出次数上限（**WS**） | 提示"本局悔棋次数已用完"或"对方不允许悔棋"。**悔棋是 WS 指令**（§7.2），REST 不承载 |
| `GAME010` | 409 | 求和次数已达上限（FR-01.10 每方每局最多 3 次）（**WS**） | 提示"本局求和次数已用完"；同上由 WS 承载 |

#### 4.2.4 `ROOM` —— 房间（8 个）

| 错误码 | HTTP | 含义 | 客户端应如何处理 |
|---|---|---|---|
| `ROOM001` | 404 | 房间不存在 | 提示"房间号不存在，请检查后重试"；**保留**输入框内容便于修正 |
| `ROOM002` | 409 | 房间已满（双方席位已被占） | 提示"房间已满"；提供"改为观战"入口（若 `allow_spectate`） |
| `ROOM003` | 409 | 房间已开局 | 提示"对局已开始"；提供"进入观战"入口 |
| `ROOM004` | 400 | 房间配置非法（时限越界、悔棋次数越界等） | 读取 `error.fields` 逐项就地标红；不重试 |
| `ROOM005` | 503 | 房间号生成冲突，请重试 | **自动重试**：最多 3 次，指数退避 100 ms / 200 ms / 400 ms；3 次后提示"创建房间失败，请稍后再试"。**这是本表中少数适合自动重试的码** |
| `ROOM006` | 409 | 观战人数已满（FR-06.05 上限 50） | 提示"观战人数已满"；把该房间在列表中置为不可进入观战 |
| `ROOM007` | 410 | 房间已关闭（空闲超时 / 对局结束后释放） | 退出房间视图，返回大厅；**不重连**该房间 |
| `ROOM008` | 409 | 未准备，无法开局 | 提示"等待双方准备"；**不**修改本地准备状态（以服务端快照为准） |

#### 4.2.5 `MATCH` —— 匹配（4 个）

| 错误码 | HTTP | 含义 | 客户端应如何处理 |
|---|---|---|---|
| `MATCH001` | 409 | 已在匹配队列中 | **不要弹错误框**。读取 `error.detail` 中的 `ticket_id` / `enqueued_at`，直接进入排队 UI（等同"重复点击开始匹配"的正常行为） |
| `MATCH002` | 409 | 不在匹配队列中（含已被匹配成功） | 检查 `error.detail.game_id`：**有值** → 跳转该对局；**无值** → 回到"开始匹配"入口并提示"匹配已取消" |
| `MATCH003` | 503 | 匹配服务暂不可用 | 按 `Retry-After` 退避重试（≤ 2 次）；同时提供"转人机对战"入口（FR-07.08 的超时保护产物） |
| `MATCH004` | 403 | 天梯模式被限制（封禁 / 反作弊标记） | 展示统一文案"该账号暂不可参与天梯匹配"（**不暴露反作弊细节**）；房间模式与人机对战仍可用 |

#### 4.2.6 `RATE` —— 限流（1 个）

| 错误码 | HTTP | 含义 | 客户端应如何处理 |
|---|---|---|---|
| `RATE001` | 429 | 请求过于频繁 | ① 读 `Retry-After`（秒）与 `X-RateLimit-Reset`；② 做**指数退避 + 抖动**重试，首次等待 ≥ 1 s；③ 对用户使用**非阻塞提示**（Toast/状态文案），**禁止**模态弹窗；④ **禁止立即重试**（会刷新窗口、延长封禁）；⑤ 写埋点上报，超阈值视为客户端 bug |

#### 4.2.7 `SYS` —— 系统（9 个）

| 错误码 | HTTP | 含义 | 客户端应如何处理 |
|---|---|---|---|
| `SYS001` | 500 | 服务内部错误 | 展示通用文案 + 展示 `request_id`（供报障）；**不自动重试**（可能已部分执行）；不崩溃、不白屏 |
| `SYS002` | 503 | 依赖服务不可用（PostgreSQL / Redis） | 按 `Retry-After` 退避重试；对局内应保持本地状态，WS 重连后走全量快照同步（[02 §6.2](02-系统架构设计.md)） |
| `SYS003` | 413 | 请求体过大 | 提示具体上限；**不**重试。客户端应在发送前就做大小校验（如棋谱导入 ≤ 64 KiB） |
| `SYS004` | 400 | 请求参数校验失败 | 读取 `error.fields` 逐项就地展示；等用户修改，**不重试** |
| `SYS005` | 400 | 不支持的协议 / 规则版本（`protocol_version` 或 `rules_version` 不匹配） | **必须**提示"客户端版本过旧，请更新后重试"（[ADR-003](14-决策记录ADR.md#adr-003)：版本不匹配则拒绝开局）；**不自动重试**（重试也不会成功） |
| `SYS006` | 404 | 资源不存在（通用兜底，如 `season_id` 不存在） | 展示"请求的资源不存在"；从当前上下文退出 |
| `SYS007` | 405 | 请求方法不支持 | **客户端 bug**（路径对但方法错），写埋点上报；不重试 |
| `SYS008` | 409 | 幂等键冲突（同一 `Idempotency-Key` 携带了**不同**的请求体） | **客户端 bug**，写埋点上报；提示"操作状态异常"并**重新生成幂等键**后允许用户重试 |
| `SYS009` | 504 | 请求处理超时（服务端内部超时） | 按 `Retry-After` 退避重试；**非幂等操作不自动重试**，先查询实际状态（避免重复扣分/重复建单） |

### 4.3 域与序号分配

| 域前缀 | 范围 | 说明 |
|---|---|---|
| `AUTH` | 鉴权、token、账号、密码 | 001..099 |
| `USER` | 资料、昵称、头像 | 001..099 |
| `GAME` | 对局、着法、棋谱 | 001..099 |
| `ROOM` | 房间生命周期、观战 | 001..099 |
| `MATCH` | 匹配队列、反作弊限制 | 001..099 |
| `RATE` | 限流（跨域统一，不分域） | 001..099 |
| `SYS` | 通用：参数、版本、幂等、超时 | 001..099 |

> **扩展规则**：新增错误码只能**追加**（用下一个未占用的序号），**禁止**复用已废弃码的含义（§9.2）。序号不必连续，但**同一域内不得有两个码含义重叠**。

### 4.4 `retryable = true` 的码

| 错误码 | 重试策略 |
|---|---|
| `AUTH002` | 先刷新 token，再重放原请求（一次） |
| `ROOM005` | 指数退避 100 / 200 / 400 ms，最多 3 次 |
| `MATCH003` | 按 `Retry-After`，最多 2 次 |
| `RATE001` | 指数退避 + 抖动，按 `Retry-After` 起算 |
| `SYS002` | 指数退避 + 抖动，上限由客户端策略决定 |
| `SYS009` | 按 `Retry-After`；**仅对幂等操作**自动重试 |

其余 40 个码**一律不自动重试**。这个白名单是刻意的窄集合——自动重试对非幂等操作会造成重复副作用，比"报错让用户决定"危险得多。

---

## 5. 分页规范

### 5.1 对局列表：游标分页（Cursor-based）

**为什么不用偏移分页**

| 问题 | 说明 |
|---|---|
| **插入导致的漂移** | 对局列表按 `created_at DESC` 排序，而**新对局不断产生**。用户翻到第 2 页时，若有 3 局新对局插入，第 2 页会重复显示第 1 页尾部的内容；反向（删除）则会漏项 |
| **深翻页的代价** | `LIMIT 20 OFFSET 10000` 需要数据库扫描并丢弃 10,000 行。象棋玩家的历史对局可达数千局，深翻是常见行为 |
| **索引效率** | 游标分页可用索引的 `(created_at, id) < ($ts, $id)` 作**范围起始**，直接定位，扫描量与 `limit` 成正比 |
| **与"加载更多"UI 天然匹配** | 战绩列表是滚动加载场景，不需要"跳到第 37 页" |

**请求参数**

| 参数 | 类型 | 必需 | 默认 | 约束 |
|---|---|---|---|---|
| `cursor` | string | 否 | — | 上一页返回的 `next_cursor`；**不透明字符串**，客户端**不得解析**其内容 |
| `limit` | integer | 否 | `20` | 1..100 |

**响应结构**

```json
{
  "items": [],
  "next_cursor": "eyJ0IjoxNzU4NjIzNzMwMDAwLCJpIjoiMDE4ZjJhM2MifQ",
  "has_more": true
}
```

| 字段 | 说明 |
|---|---|
| `items` | 数据数组，可能为空数组（`[]`），**不为 `null`** |
| `next_cursor` | 下一页游标。`has_more = false` 时为 `null` |
| `has_more` | 是否还有下一页。**客户端必须依据此字段**而非"本页条数 < limit"来判断——因为恰好整除时两者不一致 |

**游标编码**

```
cursor = base64url( "{created_at_unix_ms}:{id}" )
         ↓ 解码示例
         "1758623730000:018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f50"
```

| 约定 | 说明 |
|---|---|
| 格式 | `base64url` **不带 padding**（`=` 会被 URL 编码，去掉更干净） |
| 内容 | `<排序键原值>:<主键>`。排序键是 `created_at` 的 Unix 毫秒，主键是 UUID v7 字符串 |
| 全序保证 | 排序键 + 主键构成**全序**，因此同毫秒创建的对局不会产生重复或漏项。对应索引 `(created_at DESC, id DESC)`（[09 §3.4](09-数据模型与存储设计.md)） |
| 不透明性 | **`cursor` 的编码方式属服务端内部实现**。客户端只做"原样回传"。服务端变更编码（如加签名）不得视为破坏性变更 |
| 篡改处理 | 服务端需校验解码结果；非法 cursor 返回 `400 SYS004`，**不**静默降级为第一页（静默降级会让用户以为"数据回到开头了"） |
| 有效期 | 无过期。游标指向的 `(created_at, id)` 即使对应记录已被删除，`<` 比较仍然正确 |

**SQL 形态（参考）**

```sql
-- 首页
SELECT * FROM games
WHERE (red_user_id = $1 OR black_user_id = $1)
ORDER BY created_at DESC, id DESC
LIMIT $2;

-- 续页（$3 = 上一页最后一行的 created_at，$4 = 其 id）
SELECT * FROM games
WHERE (red_user_id = $1 OR black_user_id = $1)
  AND (created_at, id) < ($3, $4)
ORDER BY created_at DESC, id DESC
LIMIT $2;
```

> 注意：`WHERE (red_user_id = $1 OR black_user_id = $1)` 的 `OR` 会使优化器难以直接使用单列索引（[09 §3.4](09-数据模型与存储设计.md) 建了 `red` 与 `black` 两条独立索引）。**实现时应改写为 `UNION ALL` 两个分支后再排序**，否则会退化为顺序扫描。**这是必须由 `EXPLAIN` 校验的实现要求**。⚠️ 具体改写方式待实现时用 `EXPLAIN ANALYZE` 确认。

### 5.2 排行榜：偏移分页（Offset-based）

**为什么这里反而用偏移分页**

| 论点 | 说明 |
|---|---|
| **数据变化慢** | 总榜由等级分排序，等级分每局才变一次；周榜每小时才刷新一次（Redis Sorted Set 定时重建）。**不存在对局列表那种"持续插入"的漂移问题** |
| **需要跳页** | 排行榜是**浏览型**界面："我想看第 51 名到第 100 名"。游标分页无法表达"跳到第 51 名" |
| **总数 O(1)** | Redis `ZCARD` 是 O(1)（[09 §5.2](09-数据模型与存储设计.md) `leaderboard:*` 是 Sorted Set），`total` / `total_pages` 免费获得 |
| **深翻页代价可接受** | `ZRANGE ... LIMIT offset count` 在 Sorted Set 上的复杂度是 O(log(N) + offset)。N 是活跃用户量级（≤ 千万），offset 上千次跳页的代价在毫秒级 |
| **弱一致可接受** | 榜单本身就是 ≤ 60 秒的缓存快照，**不承诺与数据库强一致**（§3.7.1 `generated_at` 字段已明示）。既然不承诺一致性，"翻页完全一致"也就不是需求 |

**请求参数**

| 参数 | 类型 | 必需 | 默认 | 约束 |
|---|---|---|---|---|
| `page` | integer | 否 | `1` | ≥ 1；超过 `total_pages` 时返回**空 items 与正确分页元信息**（不报错） |
| `page_size` | integer | 否 | `50` | 1..200 |

**响应结构**

```json
{
  "items": [],
  "page": 1,
  "page_size": 50,
  "total": 12876,
  "total_pages": 258
}
```

| 字段 | 说明 |
|---|---|
| `total` | 符合条件的总条数（`ZCARD`） |
| `total_pages` | `ceil(total / page_size)`。`total = 0` 时为 `0` |
| `page > total_pages` | `items: []`，`page` 原样回显，其余元信息正常。**不返回 `404`**——"翻到空白页"是合法的用户行为，不是错误 |

### 5.3 选择判据对照

| 维度 | 游标分页 | 偏移分页 |
|---|---|---|
| 数据是否持续插入 | **是** → 游标 | 否 → 偏移可接受 |
| 是否需要跳页 | 否 → 游标 | **是** → 偏移 |
| 总数是否廉价可得 | 需要 `COUNT(*)`（昂贵） | **O(1) 可得** |
| 一致性要求 | 强（翻页不漏不重） | 弱（缓存快照即可） |
| 本项目应用 | `GET /games`（我的对局列表） | `GET /leaderboard`、`GET /lobby/games` |

> **大厅列表为什么也用偏移**：大厅是"进行中对局的实时快照"，总量小（通常 < 3,000）。它同样具备"需要跳页意义不大但排序键变化频繁"的特征——**但它有 3 秒级缓存**，缓存决定了它的一致性上限，游标带来的"强翻页一致性"没有意义。而 `total` 与排序都在 Redis Sorted Set 上（[09 §5.2](09-数据模型与存储设计.md) `lobby:ongoing`），偏移分页天然适配。

---

## 6. 限流

### 6.1 实现机制

| 项 | 约定 |
|---|---|
| 存储 | Redis，键 `ratelimit:{scope}:{key}`（[09 §5.2](09-数据模型与存储设计.md)） |
| 算法 | **固定窗口**：Lua 脚本内 `INCR` + 首次 `EXPIRE`，保证"判断与计数"原子（不可分两条命令执行） |
| 窗口对齐 | 固定窗口按 **自然时间边界**对齐（如 5 分钟窗口为 `:00`、`:05`、`:10`…），使 `X-RateLimit-Reset` 对客户端可预测 |
| 限流失败的降级 | **fail-open（放行）**。Redis 不可用时**不阻断业务**，只记 `tracing::warn!`。理由：限流是保护措施，Redis 故障时限流自身不应成为新的单点故障。登录场景另有账号锁定（`users.locked_until`）兜底 |
| 响应头 | 受限流端点必回 `X-RateLimit-Limit` / `X-RateLimit-Remaining` / `X-RateLimit-Reset`（§1.3） |
| 超限响应 | `429 RATE001` + `Retry-After` |

**为什么先选固定窗口而非滑动窗口**

| 论点 | 说明 |
|---|---|
| 成本 | 固定窗口是 1 次 `INCR`（O(1)，常数小）；滑动窗口需 ZSET 或令牌桶 Lua，内存与 CPU 都更高 |
| 精度代价已知 | 固定窗口在**窗口边界**允许最多 **2 × 阈值**的突发（跨两个窗口各用满一次）。对所有已列出的端点，这个突发量都不构成风险 |
| 有兜底 | 登录场景的**账号锁定**（§2.5）是独立于限流的第二道闸；业务侧还有幂等与唯一索引 |

> ⚠️ **待实测/校准**：若压测或线上发现边界突发被利用（如刷注册），则对 `/auth/login` 与 `/auth/register` 单独改为**滑动窗口**（Lua + ZSET），其余端点维持固定窗口。

### 6.2 限流维度

| 维度 | 键构成 | 说明 |
|---|---|---|
| IP 维度 | `ratelimit:{scope}:ip:{ip}` | `ip` 取自连接的对端地址。**信任边界**：仅在反向代理已正确处理 `X-Forwarded-For` 且代理受信时使用该头；⚠️ **具体取值来源（对端地址 vs 受信代理链）需在部署阶段确认**，取错会让所有用户共享一个 IP 桶 |
| 用户维度 | `ratelimit:{scope}:user:{user_id}` | 需鉴权端点使用；比 IP 维度更准确（同一 NAT 后的多用户不互相影响） |
| 账号维度 | `ratelimit:{scope}:account:{username}` | **仅登录使用**。按"被尝试的账号"计数，与"谁在尝试"（IP）正交 |
| 全局维度 | `ratelimit:{scope}:global` | 全站熔断（如 LLM 调用总量），本期仅在讲解相关处使用（[09 §5.2](09-数据模型与存储设计.md) `coach:llm:budget:*`） |

### 6.3 端点限流策略表

> 全部阈值为 **⚠️ 设计初始值**。推算依据列写在"依据"栏；标"无依据"的纯属经验取值，必须在上线后按实际流量分布校准。

| 端点 | 维度 | 阈值 | 窗口 | 依据 |
|---|---|---|---|---|
| `POST /auth/register` | IP | **5 次** | 1 小时 | 正常人 1 小时不会注册 5 个账号；低于此值不影响用户，高于此值不足以刷小号 ⚠️ （若发现批量注册需下调至 3） |
| `POST /auth/login` | **账号**（`username`） | **10 次** | 5 分钟 | §2.5 的锁定阈值是 5 次失败。限额设为 10，给"成功登录"留余量，同时失败 5 次即锁定，实际有效上限更低 |
| `POST /auth/login` | **IP** | **30 次** | 5 分钟 | 依据：同一 NAT（家庭/网吧）下最多约 10 台设备各尝试 3 次。低于 30 会误伤共享出口 IP 的场景 |
| `POST /auth/refresh` | 用户 | **60 次** | 1 小时 | access 有效期 900 s → 正常刷新频率 ≤ 4 次/小时（含提前刷新余量）。60 次是 15 倍余量，足以覆盖"多设备 + 时钟漂移" |
| `POST /auth/refresh` | IP | **200 次** | 1 小时 | 按"每 IP 最坏 3 个用户各 60 次"估算 |
| `POST /auth/logout` | 用户 | **30 次** | 1 小时 | 登出是低频操作；幂等（§8.2）故可放宽 |
| `POST /auth/password` | 用户 | **5 次** | 1 小时 | 改密必伴随全端下线，不应频繁；Argon2id 单次 50~100 ms，5 次/小时也限制了 CPU 放大攻击面 |
| `GET /users/me` | 用户 | **120 次** | 1 分钟 | 客户端启动与页面切换会集中调用；2 次/秒是宽松上限 |
| `GET /users/{user_id}` | 用户 / IP | **120 次** | 1 分钟 | 同上；**注意**：该端点可被用于批量枚举用户，应考虑对"不同 `user_id` 的数量"另设限（⚠️ 未在本期实现，登记为 D-09） |
| `PATCH /users/me` | 用户 | **10 次** | 1 小时 | 昵称/头像修改是低频操作，且 `nickname` 唯一索引会带来写入竞争 |
| `GET /games` | 用户 | **60 次** | 1 分钟 | 滚动加载：每页 20 条，1 次/秒已很激进 |
| `GET /games/{game_id}` | 用户 / IP | **120 次** | 1 分钟 | 复盘时逐局打开；含 `include=fens` 时服务端有计算成本，故不设更宽 |
| `GET /games/{game_id}/coach-notes` | 用户 / IP | **60 次** | 1 分钟 | 单次最多 200 条讲解，成本高于详情；1 次/秒足够 |
| `GET /users/me/stats` | 用户 | **60 次** | 1 分钟 | 涉及聚合查询 |
| `GET /lobby/games` | IP | **60 次** | 1 分钟 | 公开端点，需防爬。大厅有 2 秒缓存，实际后端压力远低于此 |
| `GET /rooms/{room_code}` | IP | **120 次** | 1 分钟 | 公开端点；房间号空间仅 10⁶，**限流是防止房间号枚举的第一道防线** |
| `GET /games/{game_id}/export` | 用户 / IP | **20 次** | 1 小时 | 导出是文件下载，成本高；正常用户不会 1 小时导出 20 次 |
| `POST /games/import` | 用户 | **30 次** | 1 小时 | 导入含逐着合法性校验（CPU 成本）；30 次/小时对批量整理棋谱的场景足够（⚠️ 若提"批量导入整本棋谱集"需求需上调） |
| `POST /match` | 用户 | **10 次** | 1 分钟 | 正常匹配不会 1 分钟点 10 次；幂等（§8.3）保证重复点击不产生多个队列成员 |
| `DELETE /match` | 用户 | **10 次** | 1 分钟 | 同上 |
| `GET /match/status` | 用户 | **60 次** | 1 分钟 | 客户端在排队期间可轮询（作为 WS 推送的兜底），1 次/秒是合理上限 |
| `GET /leaderboard` | IP | **60 次** | 1 分钟 | 公开端点 + 有 60 秒缓存 |
| `GET /seasons/current` | IP | **60 次** | 1 分钟 | 极廉价（单行查询 + 内存缓存） |
| `POST /reports` | 用户 | **5 次** | 1 小时 | 举报应慎重；另有"同一局同一理由只能一次"的唯一索引（[09 §3.9](09-数据模型与存储设计.md)） |
| `GET /healthz`、`GET /readyz` | — | **不限流** | — | 探针高频调用；且应由反向代理限制为仅内网访问 |
| **全局（兜底）** | IP | **1000 次** | 1 分钟 | 覆盖所有未单独列出的情况；也是对"新增端点忘记配置限流"的兜底 |
| **全局（兜底）** | 用户 | **600 次** | 1 分钟 | 同上 |

### 6.4 登录类端点的更严格限流

**为什么登录必须比业务端点严格**

| 攻击形态 | 特征 | 需要的维度 |
|---|---|---|
| **横向撞库** | 固定少数常见密码 × **大量用户名**，用同一 IP 或 IP 池 | **IP 维度**（限制单来源速率）+ **账号维度**（限制单账号被尝试次数） |
| **纵向爆破** | **单个目标账号** × 大量密码 | **账号维度**是唯一有效的维度（IP 会轮换） |
| **凭据填充** | 从其他站点泄露的 `用户名+密码` 对批量尝试 | 账号维度 + 账号锁定 |
| **注册灌水** | 批量注册小号 | IP 维度 + 用户名唯一索引 |

因此登录采用**三重限流 + 一重锁定**：

| 层次 | 机制 | 阈值 |
|---|---|---|
| 第 1 层 | IP 限流 | 30 次 / 5 分钟 |
| 第 2 层 | 账号限流 | 10 次 / 5 分钟 |
| 第 3 层 | **账号连续失败锁定** | 连续 5 次密码错 → `locked_until = now() + 15 min`（[09 §3.1](09-数据模型与存储设计.md)）⚠️ 阈值与时长待校准 |
| 第 4 层 | Argon2id 计算成本 | 单次哈希 50~100 ms（§2.5），使离线爆破与在线爆破的速率都被物理限制 |

**与业务端点的量级对比**：业务端点最严的是 `POST /auth/password`（5 次/小时）与 `POST /reports`（5 次/小时），登录的 **IP 30 次/5 分钟 = 360 次/小时** 看似更宽——但登录叠加了**账号维度**（同一目标账号 10 次/5 分钟 = 120 次/小时，且失败 5 次即锁 15 分钟 → 实际有效尝试 ≤ 60 次/小时且此后窗口内完全阻断）。**登录的严格性体现在"多重维度叠加"而非单一阈值更低**。

**未做但记录在案的加固项**（登记为 §12 的 D 系列）：

| 项 | 说明 |
|---|---|
| CAPTCHA | 连续失败 3 次后要求人机验证。**本期不做**（需引入第三方服务，[01 §7](01-需求规格说明书.md) 需求边界未列） |
| 异地登录检测 | 基于 `register_ip` / `last_login_ip` 的地理解析。**本期不做**（需要 IP 地理库） |
| 泄露密码库比对 | 与已知泄露密码哈希集比对。**本期不做**（需外部数据源） |

### 6.5 超限响应示例

```json
HTTP/1.1 429 Too Many Requests
Retry-After: 187
X-RateLimit-Limit: 10
X-RateLimit-Remaining: 0
X-RateLimit-Reset: 1758625000
Content-Type: application/json; charset=utf-8

{
  "error": {
    "code": "RATE001",
    "message": "操作过于频繁，请在 3 分钟后重试。",
    "detail": {
      "scope": "login_account",
      "limit": 10,
      "window_seconds": 300,
      "retry_after_ms": 187000
    },
    "fields": null,
    "request_id": "018f2a3c-7d1e-7b3a-9c4f-2a6b8d0e1f91",
    "retryable": true
  }
}
```

> `error.detail` 中给出的 `scope` 与阈值**不构成信息泄露风险**（攻击者本就能通过试错探知），而它们对合法客户端的退避逻辑有实质帮助。

### 6.6 WebSocket 侧的限流

WS 帧的限流不在本文范围（帧定义属 [06-联网协议](06-联网对战与实时通信协议.md)），但**限流键规范一致**，在此仅登记约定以便实现统一：

| 场景 | 维度 | 阈值 | 依据 |
|---|---|---|---|
| WS 新建连接 | IP | **20 次/分钟** | 断线重连的指数退避最大 30 s → 正常 ≤ 2 次/分钟，20 次留 10 倍余量（[02 §6.2](02-系统架构设计.md) 退避 1/2/4/8/16/30 s） |
| **走子帧** | 用户 | **20 帧 / 10 秒** ⚠️ | 依据：人类极限约 1 步/秒（快速走子），留 2 倍余量。超限视为异常行为，记入反作弊（NFR-S.08） |
| 聊天帧 | 用户 | **5 帧 / 10 秒** ⚠️ | FR-05.09 聊天；刷屏防护 |
| 其他对局指令（认输/求和/悔棋） | 用户 | **10 帧 / 分钟** ⚠️ | 均为低频深思熟虑的操作 |
| 观战订阅 | 用户 | **10 次/分钟** ⚠️ | 防止频繁切换房间 |

---

## 7. REST 与 WebSocket 的职责边界

### 7.1 判据

[02 §5.1](02-系统架构设计.md) 已确立双通道设计：

> **REST (HTTPS)**：账号、资料、战绩、大厅列表、棋谱读写 —— 无状态、可缓存、幂等、易限流
> **WebSocket**：对局实时帧、观战、匹配队列、在线状态 —— 全双工、低延迟、有状态会话

本文在此基础上给出**可执行的判据**，用于裁决"一个新功能该走哪个通道"：

| 判据 | 走 REST | 走 WS |
|---|---|---|
| ① 是否需要**服务端主动推送** | 否 | **是**（服务端要主动发消息） |
| ② 是否属于**长时间会话**的一部分 | 否 | **是**（连接生命周期内有多次交互） |
| ③ 是否要求**极低延迟**（< 1 个 RTT 可接受？） | 可接受 | **不可接受**（必须走已建立的连接） |
| ④ 是否**幂等 / 可重试** | 是 → REST 天然契合 | 否（WS 无请求-响应配对语义） |
| ⑤ 是否需要**断线后独立可查询** | **是** → REST | 否 |
| ⑥ 是否需要**可缓存** | **是** → REST | 否 |

**综合判据**：以上 6 条**任一条倾向 WS 且无强反向理由** → 走 WS；否则走 REST。

### 7.2 边界总表

| 操作 | 通道 | 判据命中 | 说明 |
|---|---|---|---|
| 注册 / 登录 / 刷新 / 登出 / 改密 | **REST** | ④⑤⑥ | 账号操作天然幂等或可重试；登出后需独立可查询状态 |
| 查询/修改资料 | **REST** | ④⑥ | 可缓存、可重试 |
| 我的对局列表 / 详情 / 统计 | **REST** | ④⑤⑥ | 分页查询、可缓存 |
| 棋谱导出 / 导入 | **REST** | ④⑥ | 纯数据变换，无副作用（导入）或只读（导出） |
| 大厅进行中对局列表 | **REST** | ④⑥ | 可缓存（2 秒）；轮询频率低 |
| 房间信息（**只读**）查询 | **REST** | ④⑥ | 见 §3.4.2 关于"未开局房间"的说明 |
| 匹配**指令**（开始/取消/查询） | **REST** | ④⑤ | 见 §7.3 的细化说明 |
| 匹配**结果推送**（`MatchFound`） | **WS** | ①②③ | 服务端主动推、需低延迟 |
| 排行榜 / 赛季信息 | **REST** | ④⑥ | 可缓存 |
| 举报提交 | **REST** | ④⑤ | 幂等（同一局同一理由一次）+ 需独立可查询 |
| 健康检查 | **REST** | ④ | 探针是 HTTP 语义 |
| **房间创建 / 加入 / 离开** | **WS** | ①②③ | 见 §7.4 |
| **走子**（`MoveRequest`） | **WS** | ①②③ | 见 §7.4。**必须**走 WS——这是 NFR-P.05（≤150 ms）与 [ADR-003](14-决策记录ADR.md#adr-003) 乐观更新的前提 |
| 认输 / 求和 / 悔棋 / 聊天 | **WS** | ①②③ | 房间会话内的指令，需即时广播对局双方 |
| 观战订阅 / 取消订阅 | **WS** | ①②③ | 需要持续接收帧 |
| 讲解帧（`CoachNote`）下发 | **WS** | ①②③ | 服务端异步推送（[ADR-004](14-决策记录ADR.md#adr-004)） |
| 时钟同步（`ClockSync`） | **WS** | ①②③ | [ADR-014](14-决策记录ADR.md#adr-014) 要求定期下发 |
| 在线状态（隐藏/离开） | **WS** | ①② | 状态变化即推送 |

### 7.3 与 [02 §5.1](02-系统架构设计.md) 的关系说明（细化，非冲突）

[02 §5.1](02-系统架构设计.md) 把"匹配队列"归入 WebSocket。本文**细化为**：**匹配指令走 REST，匹配结果走 WS**。理由：

| 论点 | 说明 |
|---|---|
| 语义是"提交意图 + 等待结果" | 匹配不是持续的双向流，而是"我提交一个排队请求 → 等系统分配对手"。这是典型的**请求-响应 + 异步回调**语义，天然对应"REST 提交 + WS 回调" |
| 需要幂等与可重试 | "开始匹配"必须能被安全重试（网络抖动时客户端不知道是否提交成功）。REST 的 `Idempotency-Key`（§8.3）与 `MATCH001` 的幂等友好处理正好覆盖这个需求；WS 帧没有请求-响应配对语义，无法表达"这次重复提交是重试而非第二次排队" |
| 需要断线后可查询 | 客户端在排队期间断线（WS 断开），重连后 WS 上没有任何"当前是否在队列中"的信息。`GET /match/status`（§3.6.3）提供了**独立的权威状态源**，这是 WS 无法替代的 |
| 与 [ADR-010](14-决策记录ADR.md#adr-010) 一致 | 匹配队列在 Redis（`match:queue`），REST 与 WS 两种入口读写的是**同一份队列数据**，通道选择不影响数据模型 |

> **实施约定**：WS 侧的 `MatchJoin` / `MatchLeave` 帧**保留定义但不推荐使用**（[06-联网协议](06-联网对战与实时通信协议.md) 可将其标记为 `deprecated`）。客户端应统一使用 REST 指令 + WS `MatchFound` 推送的组合。

### 7.4 为什么房间创建/加入与走子必须走 WS

| 操作 | 走 WS 的理由 |
|---|---|
| **房间创建 / 加入** | ① 房间生命周期极短（空闲超时 900 s，[02 §8](02-系统架构设计.md) `room_idle_timeout_secs`），为它建 REST 资源与 PG 记录是过度设计（§3.4.2 已说明未开局房间不落库）；② 加入房间后**立即**需要在该连接上收发对局帧，"先 REST 加入、再 WS 连接"会引入"加入成功但 WS 未连通"的中间态，需要额外的补偿逻辑；③ 加入的结果（成功/房间满/已开局）需要在**已建立的连接**上立即得知（`ROOM002` / `ROOM003` 通过 `ErrorFrame` 返回） |
| **走子** | ① **延迟要求**：NFR-P.05 要求端到端 ≤ 150 ms，[ADR-003](14-决策记录ADR.md#adr-003) 要求本地乐观更新后立即发送。走 REST 会在 WS 之外再建一条 TCP/TLS 通道（或复用连接但走新的 HTTP 事务），增加握手与队头阻塞风险；② **广播语义**：一次走子需要广播给对局双方与全部观战者——这正是 WS 房间订阅模型的核心能力；③ **顺序保证**：走子必须严格有序，同一 WS 连接的**消息顺序天然保序**，而多个 HTTP 请求可以并发乱序到达（需额外用 `seq` 排序）；④ **服务端主动回推**：裁决失败要主动推 `MoveRejected` + 全量快照，REST 无法主动推 |

> **通道与幂等的关系**：走子走 WS **不意味着放弃幂等**。走子的幂等由 WS 帧的 `client_seq`（[02 §4.1](02-系统架构设计.md) `RoomCommand::Move { client_seq }`）+ 数据库 `(game_id, seq)` 唯一索引（[09 §3.5](09-数据模型与存储设计.md)）共同保证，见 §8.3。

---

## 8. 幂等性

### 8.1 需要幂等的端点

| 端点 | 是否幂等 | 实现层次 | 说明 |
|---|---|---|---|
| `POST /auth/register` | **是** | 数据库唯一索引（`users.username`） | 重复提交返回 `AUTH008`（不是静默成功——注册场景需要明确告知"这个名字已被占了，可能是你刚才提交的"） |
| `POST /auth/login` | 不适用 | — | 每次登录产生新 token 对，重复登录是合法行为（多设备） |
| `POST /auth/refresh` | **否（刻意不幂等）** | 轮换 + 复用检测（§2.3） | 重复刷新会被视为复用并**撤销全部会话**。这是安全特性，不是缺陷。客户端必须实现单飞（§2.3） |
| `POST /auth/logout` | **是** | 天然（`DEL` 幂等） | 重复登出均返回 `204` |
| `POST /auth/password` | **否** | — | 第二次因旧密码已变而失败（`AUTH004`），这是正确行为 |
| `GET *` | **是** | 安全方法 | 无副作用 |
| `PATCH /users/me` | **是** | 语义幂等（"设为该值"） | 重复提交结果相同，无需幂等键 |
| `POST /match` | **是** | Redis `SET NX match:queue:member:{user_id}` + `MATCH001` 幂等友好响应 | **必须幂等**：客户端在网络抖动时无法知道"入队是否成功"，重试是必然行为。见 §8.3 |
| `DELETE /match` | **是** | `ZREM` 幂等 | 不在队列中也返回 `204`（但"已被匹配成功"时返回 `MATCH002`——因为此时取消**不可能**成功，必须告知） |
| `POST /games/import` | **是** | 纯计算无副作用 | 幂等键可选；重复导入返回相同解析结果 |
| `GET /games/{id}/export` | **是** | 安全方法 | — |
| `POST /reports` | **是** | 数据库部分唯一索引（`reports_dedup_idx`，[09 §3.9](09-数据模型与存储设计.md)） | 重复提交返回 `200` + 原工单（§3.8.1） |
| **走子（WS 帧）** | **是** | `client_seq` + `(game_id, seq)` 唯一索引 | 见 §8.3。**这是全项目最关键的幂等要求**——重复走子会破坏对局 |
| 认输 / 求和（WS 帧） | **是** | 天然（"已经认输了再认输一次" = 无变化） | 服务端应静默忽略重复的认输 |
| 悔棋（WS 帧） | **否** | — | 每次悔棋是独立请求，需要对方确认；重复提交会消耗一次悔棋机会，因此需要 `client_seq` 去重 |

### 8.2 幂等键机制（REST）

**适用端点**：`POST /match`、`POST /reports`（其余端点或天然幂等、或不应幂等）。

**协议**

| 项 | 约定 |
|---|---|
| 请求头 | `Idempotency-Key: <UUID v7 字符串>` |
| 服务端存储 | Redis `idempotency:{scope}:{key}` = `{request_hash}`，TTL **300 秒** ⚠️ 设计初始值（[09 §5.2](09-数据模型与存储设计.md)） |
| `scope` | 端点标识（如 `match.create`、`report.create`），防止跨端点的键碰撞 |
| `request_hash` | 请求体的规范化哈希（如 SHA-256 over canonical JSON，**排除**时间戳类字段）。用于识别"同键不同体" |
| 首次请求 | `SET idempotency:{scope}:{key} {request_hash} NX EX 300` 成功 → 正常执行业务逻辑 → **把响应体也缓存**（`idempotency:{scope}:{key}:resp`，同 TTL） |
| 重复请求（同键同体） | `SET NX` 失败且 `request_hash` 相同 → **直接返回缓存的首个响应**（相同状态码与响应体），不重复执行业务逻辑 |
| 重复请求（同键**不同体**） | `request_hash` 不同 → `409 SYS008`（客户端 bug：同一个幂等键必须对应同一个请求） |
| 未携带幂等键 | 按非幂等请求处理（正常执行）。**不报错**——幂等键是可选优化，不应成为使用门槛 |
| 首次请求执行中就被重试 | 并发场景：第二个请求 `SET NX` 失败但 `:resp` 尚不存在 → 返回 `409 SYS008` 或 `425 Too Early`？**设计选择**：返回 `409 SYS008`，并在 `detail` 中标注 `"in_flight": true`。客户端应短暂等待后重试（此时结果已缓存）。⚠️ 更优雅的方案是"占位 + 轮询"，属过度设计，本期不做 |

**为什么只用 Redis 而不落库**

| 论点 | 说明 |
|---|---|
| 幂等键是**短期**语义 | 300 秒足够覆盖网络重试窗口（客户端退避 ≤ 30 s）；长期幂等应由**数据库唯一索引**承担（§8.4） |
| 落库会污染数据模型 | 为了幂等键建一张表，其行数会超过 `games`（每次点击一行），收益与成本严重不成比例 |
| 丢失影响可控 | Redis 丢失 → 幂等保证降级为"数据库唯一索引兜底"（§8.4），不会产生重复数据，最多产生一次 `409` 冲突响应 |

### 8.3 走子与匹配的幂等实现

#### 走子（WS 帧）

```
客户端                          服务端（房间 Actor）
  │
  ├─ 本地 xq-core 校验 → 立即渲染（乐观更新）
  ├─ 记录 client_seq = 本地递增计数器（从 1 开始，单调不减）
  ├─ MoveRequest { mv, client_seq, last_server_seq }
  │
  │                             ├─ 若 client_seq ≤ 已处理的最大 client_seq
  │                             │   → 静默丢弃（重复帧），不广播、不回错误
  │                             ├─ 若 last_server_seq ≠ 当前服务端 seq
  │                             │   → 返回 MoveRejected{reason: "stale"} + 全量快照
  │                             ├─ 权威裁决（xq-core）
  │                             ├─ 服务端分配 seq = 当前着数 + 1
  │                             ├─ XADD game:{id}:moves  ← (game_id, seq) 唯一索引兜底
  │                             └─ 广播 MoveApplied{seq, mv, clock, ...}
  │
  └─ 收到 MoveApplied → 用服务端 seq 校正本地计数器
```

| 幂等层次 | 机制 | 防什么 |
|---|---|---|
| ① 帧级 | `client_seq` 单调递增；服务端丢弃 `client_seq ≤ last` | WS 重连后客户端重发未确认的帧 |
| ② 状态级 | `last_server_seq` 不匹配则拒绝并要求全量同步（[02 §6.2](02-系统架构设计.md)） | 客户端与房间状态分歧 |
| ③ 存储级 | `game_moves_game_seq_key (game_id, seq)` 唯一 + `ON CONFLICT DO NOTHING`（[ADR-011](14-决策记录ADR.md#adr-011)） | 落库消费者重放导致的重复行 |

> **三层是互补的**：① 处理"网络层重发"，② 处理"状态不一致"，③ 处理"消费者重放"。缺任何一层都会在对应场景下出问题。

#### 匹配（REST）

| 步骤 | 动作 | 幂等效果 |
|---|---|---|
| 1 | 校验用户不在队列：`EXISTS match:queue:member:{user_id}` | 命中 → 返回 `409 MATCH001` + 已有 `ticket_id`（§4.2.5 约定客户端直接进入排队 UI） |
| 2 | `SET match:queue:member:{user_id} {ticket_id} NX EX 600` | 并发重复提交时只有一个成功；失败者回退到步骤 1 的响应 |
| 3 | `ZADD match:queue {rating} {user_id}` | `ZADD` 本身幂等（同 member 重复添加只是更新 score） |
| 4 | 返回 `202` + `ticket_id` | — |

> **`ticket_id` 的生成时机**：在步骤 2 的 `SET NX` **之前**生成并写入值。这样并发场景下，两个请求各自生成一个 `ticket_id`，但只有一个人的 `SET NX` 成功——失败者读到的是**成功者的 `ticket_id`**，因此返回给客户端的票据是**唯一且正确**的。

### 8.4 幂等的兜底层次

| 层次 | 机制 | 覆盖场景 | 丢失/失效影响 |
|---|---|---|---|
| L1 客户端 | 幂等键 / `client_seq` / 单飞刷新 | 网络重试 | 客户端 bug → 退到 L2 |
| L2 中间件 | Redis `idempotency:*` 缓存响应 | REST 重复提交 | Redis 故障 → 退到 L3（`fail-open`，见 §6.1） |
| L3 业务 | Redis `SET NX`（匹配成员键）、房间 Actor 状态判断 | 并发重复 | Redis 故障 → 退到 L4 |
| L4 **数据库** | **唯一索引 + `ON CONFLICT`**：`users.username`、`game_moves(game_id, seq)`、`game_coach_notes(game_id, ply)`、`rating_history(game_id, user_id)`、`reports(reporter_id, target_game_id, reason)`、`user_season_stats(user_id, season_id)` | **所有场景** | **无失效可能**（[ADR-009](14-决策记录ADR.md#adr-009) 的持久化保证） |

> **设计原则**：**正确性只依赖 L4，体验依赖 L1~L3。** 任何 L1~L3 的失效最多导致"多一次往返 / 多一次报错"，不会产生重复数据。这也是为什么 [09 §4.1](09-数据模型与存储设计.md) 强调"用唯一索引承载幂等"。

---

## 9. 版本演进

### 9.1 `/api/v1` 兼容策略

| 规约 | 内容 |
|---|---|
| **路径版本** | 主版本号在路径中：`/api/v1`。同时只维护**一个**主版本（当前 `v1`） |
| **只增不减** | `v1` 生命周期内：**允许**新增端点和新增**可选**响应字段；**禁止**删除端点、删除字段、改字段类型、改字段语义、收紧已有字段的校验 |
| **客户端必须忽略未知字段** | 客户端反序列化必须容忍多余字段（Rust 端 serde 默认行为；JS 端天然忽略）。这样服务端可以先行上线新字段（[ADR-007](14-决策记录ADR.md#adr-007) 后果栏同理：WS 未知帧必须可优雅忽略） |
| **枚举必须支持未知值** | 客户端遇到未识别的枚举值时，必须降级为**通用展示**而非崩溃。例：新的 `end_reason` 取值出现时，旧客户端应显示"对局结束"而非解析失败 |
| **可选字段默认值** | 新增请求字段必须可选且有服务端默认值；新增响应字段必须始终存在（可以为 `null`） |
| **什么算破坏性变更** | ① 删除/重命名端点或字段；② 字段类型变更（`string` → `integer`）；③ 语义变更（`rating` 从"当前分"改为"赛季分"）；④ 必填性收紧（可选 → 必填）；⑤ 枚举值含义变更；⑥ 错误码含义变更；⑦ 分页方式变更（游标 ↔ 偏移） |
| **破坏性变更的处理** | 新增 `/api/v2`，`v1` 与 `v2` **并行运行**。共享的底层逻辑（`xq-core` / 仓储层）不变，只有 DTO 与路由分层 |

### 9.2 废弃流程

```mermaid
graph LR
    A["T0 宣布废弃<br/>文档标注 + 公告"] --> B["T0+30d 开始回告警<br/>响应头 Deprecation/Sunset"]
    B --> C["T0+60d 灰度监控<br/>统计 v1 端点调用量"]
    C --> D{"调用量是否归零？"}
    D -->|否| E["延长 Sunset 并联系重度调用方"]
    D -->|是| F["T0+180d 下线<br/>返回 410 + 迁移指引"]
```

| 阶段 | 服务端动作 | 客户端可观测信号 |
|---|---|---|
| T0（宣布） | 文档标 `已废弃`；变更日志公告 | 无（仅文档） |
| T0 + 30 天 | 响应头加 `Deprecation: true`、`Sunset: <RFC 3339 时间>`、`Link: </api/v2/xxx>; rel="successor-version"` | 响应头 |
| T0 + 60 天 | 开始记录废弃端点调用量并出报表 | 无变化 |
| T0 + 180 天 | 端点返回 `410 Gone`（**不是 `404`**，明确表达"曾经存在但已移除"） | `410` + `error.message` 给出迁移指引 |

| 参数 | 取值 | 说明 |
|---|---|---|
| 最短废弃期 | **180 天** ⚠️ 设计初始值 | 桌面客户端用户可能长期不更新。若上线后数据显示更新率低，应延长 |
| 保留端点数量 | 至多 2 个主版本并行 | 避免维护矩阵爆炸 |
| 错误码**永不**复用 | 废弃的码标记为 `deprecated` 但**不删除、不复用序号** | 老客户端可能仍在解析该码 |

### 9.3 WS 协议版本协商

[ADR-003](14-决策记录ADR.md#adr-003) 要求"握手时进行**协议版本 + 规则版本**协商，不匹配则拒绝开局"。具体机制：

| 项 | 约定 |
|---|---|
| 协商时机 | WS 升级后的**第一帧**（`Hello` 帧，[06-联网协议](06-联网对战与实时通信协议.md) 定义） |
| 客户端提供 | `protocol_version`（协议版本字符串，如 `"1.3"`）+ `rules_version`（规则版本，如 `"2026.1"`） |
| 服务端支持窗口 | `[MIN_SUPPORTED_PROTOCOL, CURRENT_PROTOCOL]`（如 `["1.0", "1.3"]`）+ 规则版本需**精确匹配** |
| 匹配成功 | 服务端回 `HelloAck`，随后进入正常帧流 |
| `protocol_version` 低于最小值 | 关闭连接，关闭码 **4001**，并先发送 `ErrorFrame{code: "SYS005"}`（`retryable: false`，`detail` 含 `min_supported` 与 `client_version`） |
| `protocol_version` 高于当前（客户端新于服务端） | 允许连接，但服务端只发 `CURRENT_PROTOCOL` 支持的帧。客户端必须优雅忽略未知帧（[ADR-007](14-决策记录ADR.md#adr-007)） |
| **`rules_version` 不匹配** | **拒绝开局**（`SYS005`），但允许连接与观战。理由：规则版本不一致会导致"客户端认为合法、服务端判非法"（[ADR-003](14-决策记录ADR.md#adr-003) 权衡栏），但对局中若双方规则版本不同则必须阻断 |
| 版本上报 | `Hello` 帧中同时上报客户端版本号（如 `"0.4.2"`），写入会话上下文与 `audit_logs`，便于按版本分析问题 |
| REST 侧 | `rules_version` 通过响应体字段返回（如 `GET /games/{id}` 的 `rules_version`、`POST /games/import` 的 `rules_version`），客户端可比对并提示"规则已更新" |

---

## 10. 跨域与安全头

### 10.1 CORS 策略

**关键事实**：本项目的客户端是 **Tauri 桌面应用**，其前端（React）通过 **Tauri IPC** 调用 Rust 侧（[02 §2](02-系统架构设计.md) 桥接层），REST/WS 请求由 **Rust 侧发起**（`xq-client::net`）。**Rust 的 HTTP 客户端不受同源策略约束**，因此**生产环境下前端不产生跨域请求**。

| 环境 | `Access-Control-Allow-Origin` | 说明 |
|---|---|---|
| 生产 | **不返回 CORS 头**（默认拒绝） | 桌面客户端不需要 CORS。返回 `*` 只会扩大浏览器攻击面 |
| 开发 | 白名单精确匹配配置项 `server.cors_allowed_origins`，默认 `["http://localhost:5173"]`（Vite dev server） | 便于在浏览器中调试 REST 接口（前端独立起 Vite 而不启动 Tauri） |

| CORS 规则 | 取值 | 理由 |
|---|---|---|
| `Access-Control-Allow-Origin` | **精确回显白名单中的 Origin**，绝不使用 `*` | `*` 与凭据共存时浏览器会拒绝，且 `*` 会让任意站点读取响应 |
| `Access-Control-Allow-Credentials` | **不设置** | 本项目用 `Authorization: Bearer` 头传 token，**不用 Cookie**。因此**不存在 CSRF 问题**，也无需 `credentials` |
| `Access-Control-Allow-Methods` | `GET, POST, PATCH, DELETE, OPTIONS` | 与实际使用的方法集合一致 |
| `Access-Control-Allow-Headers` | `Authorization, Content-Type, X-Request-Id, Idempotency-Key` | 最小必要集合 |
| `Access-Control-Expose-Headers` | `X-Request-Id, X-RateLimit-Limit, X-RateLimit-Remaining, X-RateLimit-Reset, Retry-After` | **必须显式暴露**，否则浏览器环境下的前端读不到这些头 |
| `Access-Control-Max-Age` | `600` ⚠️ 设计初始值 | 预检请求缓存 10 分钟 |
| `Vary` | `Origin` | 有 CORS 头时必须带，避免缓存串源 |

**WS 升级的跨域防护**：浏览器在 WS 升级时会带 `Origin` 头。服务端**必须校验**：仅接受白名单内的 Origin（生产环境下桌面客户端的 WS 客户端通常不带 `Origin`，此时放行；带 `Origin` 且不在白名单则拒绝，关闭码 4003）。

### 10.2 安全响应头

| 响应头 | 取值 | 理由 |
|---|---|---|
| `Strict-Transport-Security` | `max-age=31536000; includeSubDomains` | 强制 HTTPS（1 年）。⚠️ 首次上线应先短时长（如 300 s）验证，再逐步加长；**确认全站 HTTPS 已完成**后才可开启 `includeSubDomains` |
| `X-Content-Type-Options` | `nosniff` | 禁止浏览器对 `application/json` 做 MIME 嗅探后当脚本执行 |
| `X-Frame-Options` | `DENY` | API 响应不应被任何页面 iframe 嵌入 |
| `Referrer-Policy` | `no-referrer` | API 响应不携带 Referer 信息外泄 |
| `Content-Security-Policy` | `default-src 'none'; frame-ancestors 'none'; base-uri 'none'` | API 只返回 JSON，**不需要加载任何资源**。最严格的 CSP 是正确选择（若某天需要返回 HTML 错误页，需先放宽此项） |
| `Cache-Control` | 鉴权相关端点：`no-store`；公开列表端点：`public, max-age=2`（大厅）/ 无（其余默认 `no-cache`） | 防止 token 相关响应被中间代理缓存 |
| `Permissions-Policy` | `geolocation=(), camera=(), microphone=(), payment=()` | 明确关闭不需要的浏览器能力 |
| `Server` | **不返回**或返回固定字符串 | 不泄露服务器软件与版本 |

**传输安全**

| 项 | 约定 |
|---|---|
| TLS 版本 | **TLS 1.2+**，优先 **TLS 1.3**。禁止 TLS 1.0/1.1 |
| 密码套件 | 仅启用 AEAD 套件（AES-GCM / ChaCha20-Poly1305），禁止 CBC 与 RC4 |
| 证书 | 由反向代理（Nginx / Caddy，[02 §7.2](02-系统架构设计.md)）终止 TLS；`xq-server` 只监听内网明文 HTTP |
| HTTP → HTTPS | 反向代理层 301 重定向 |
| WS | `wss://`（TLS 与 HTTP 同端口，由代理的 Upgrade 透传处理，[02 §7.2](02-系统架构设计.md)） |

### 10.3 请求体大小限制

| 端点类别 | 上限 | 依据 |
|---|---|---|
| 默认（全局） | **64 KiB** | 所有 JSON 请求体中最大的合法载荷是棋谱（`POST /games/import` 的 `content`，最长对局 240 着 × 每着 20 字符 ≈ 5 KiB；含注释与 PGN 头后 < 20 KiB）。64 KiB 是 3 倍余量 |
| `POST /games/import` | **64 KiB** | 同上 |
| `POST /reports` | **16 KiB** | `detail` ≤ 1000 字符（3 KiB）+ `evidence` 白名单字段（< 1 KiB），16 KiB 是 4 倍余量 |
| `PATCH /users/me` | **1 KiB** | 昵称 ≤ 24 字符 + `avatar_id` ≤ 64 字符 |
| 其他写端点 | **8 KiB** | 覆盖房间配置、匹配偏好等 |
| 超限响应 | `413 SYS003` | 反向代理层也应配置同等的 `client_max_body_size`，避免大包进入应用层 |

**实现约定**：用 `tower_http::limit::RequestBodyLimitLayer`（[README §3.2](../README.md) `tower-http` 0.7.1）设置全局默认，用 `axum::extract::DefaultBodyLimit` 对个别端点覆写。

---

## 11. 端点总表

| # | 方法 | 路径 | 鉴权 | 幂等 | 限流维度 | 成功码 | 分页 |
|---|---|---|---|---|---|---|---|
| 1 | POST | `/api/v1/auth/register` | 公开 | 唯一索引 | IP 5/h | `201` | — |
| 2 | POST | `/api/v1/auth/login` | 公开 | 不适用 | 账号 10/5min + IP 30/5min | `200` | — |
| 3 | POST | `/api/v1/auth/refresh` | 公开（凭 refresh） | **否**（轮换） | 用户 60/h + IP 200/h | `200` | — |
| 4 | POST | `/api/v1/auth/logout` | 必需 | **是** | 用户 30/h | `204` | — |
| 5 | POST | `/api/v1/auth/password` | 必需 | 否 | 用户 5/h | `200` | — |
| 6 | GET | `/api/v1/users/me` | 必需 | 是 | 用户 120/min | `200` | — |
| 7 | GET | `/api/v1/users/{user_id}` | 可选 | 是 | 用户 120/min | `200` | — |
| 8 | PATCH | `/api/v1/users/me` | 必需 | **是** | 用户 10/h | `200` | — |
| 9 | GET | `/api/v1/users/me/stats` | 必需 | 是 | 用户 60/min | `200` | — |
| 10 | GET | `/api/v1/games` | 必需 | 是 | 用户 60/min | `200` | **游标** |
| 11 | GET | `/api/v1/games/{game_id}` | 可选 | 是 | 用户 120/min | `200` | — |
| 12 | GET | `/api/v1/games/{game_id}/coach-notes` | 可选 | 是 | 用户 60/min | `200` | 区间（`from_ply`/`to_ply`） |
| 13 | GET | `/api/v1/games/{game_id}/export` | 可选 | 是 | 用户 20/h | `200`（text/plain） | — |
| 14 | POST | `/api/v1/games/import` | 必需 | **是** | 用户 30/h | `200` | — |
| 15 | GET | `/api/v1/lobby/games` | 公开 | 是 | IP 60/min | `200` | **偏移** |
| 16 | GET | `/api/v1/rooms/{room_code}` | 公开 | 是 | IP 120/min | `200` | — |
| 17 | POST | `/api/v1/match` | 必需 | **是** | 用户 10/min | `202` | — |
| 18 | DELETE | `/api/v1/match` | 必需 | **是** | 用户 10/min | `204` | — |
| 19 | GET | `/api/v1/match/status` | 必需 | 是 | 用户 60/min | `200` | — |
| 20 | GET | `/api/v1/leaderboard` | 公开 | 是 | IP 60/min | `200` | **偏移** |
| 21 | GET | `/api/v1/seasons/current` | 公开 | 是 | IP 60/min | `200` | — |
| 22 | POST | `/api/v1/reports` | 必需 | **是** | 用户 5/h | `201` | — |
| 23 | GET | `/healthz` | 公开（内网） | 是 | 不限流 | `200` | — |
| 24 | GET | `/readyz` | 公开（内网） | 是 | 不限流 | `200` / `503` | — |
| — | 全局兜底 | 所有路径 | — | — | IP 1000/min + 用户 600/min | — | — |

**按域统计**

| 域 | 端点数 |
|---|---|
| 账号 `/auth` | 5 |
| 资料 `/users` | 4（含 `/users/me/stats`） |
| 战绩 `/games` | 5（列表、详情、讲解、导出、导入） |
| 大厅 `/lobby` + `/rooms` | 2 |
| 匹配 `/match` | 3 |
| 排行榜 + 赛季 | 2 |
| 举报 `/reports` | 1 |
| 运维（不在 `/api/v1` 下） | 2 |
| **合计** | **24** |

---

## 12. 本文标注的「待实测 / 待校准 / 待确认」项清单

> 按硬约束要求，以下均为**设计初始值**或**待决事项**，未经实测、校准或产品确认，**不得作为验收结论或对外承诺**。

### L 系列 —— 有效期与阈值

| 编号 | 项 | 本文取值 | 校准方法 |
|---|---|---|---|
| L-01 | access token 有效期 | 900 s | 结合"登出后残留窗口"的可接受度与刷新频率对 Redis 的压力实测 |
| L-02 | refresh token 有效期 | 2,592,000 s（30 天） | 桌面客户端的实际使用间隔统计 |
| L-03 | 客户端提前刷新窗口 | 60 s | 实测因"途中过期"导致的 `AUTH002` 比例 |
| L-04 | token 时钟偏移容忍 | 30 s | 实测客户端与服务端的时钟漂移分布 |
| L-05 | Argon2id 参数（`m=19456 KiB, t=2, p=1`） | 见 §2.5 | **必须实测单次哈希耗时**，目标 50~100 ms；超目标下调 `m` |
| L-06 | 密码长度上限 | 64 | 压测超长密码对 Argon2id 与日志的影响 |
| L-07 | 弱密码表规模 | ≥ 10,000 条 | 取公开弱密码字典（需确认许可）后固化 |
| L-08 | 登录失败锁定阈值 / 时长 | 5 次 / 15 分钟 | 上线后统计误锁率与撞库拦截率 |
| L-09 | 账号限流 10 次/5 分钟 | 见 §6.3 | 同上 |
| L-10 | IP 限流 30 次/5 分钟 | 见 §6.3 | 统计数据中心 NAT / 校园网出口的共享 IP 占比，评估误伤 |
| L-11 | 全部端点限流阈值 | 见 §6.3 | 上线后按 `429` 比率（目标 < 0.1%）与恶意流量拦截率双向校准 |
| L-12 | 固定窗口是否需改滑动窗口 | 固定窗口 | 压测边界突发；若有刷注册/刷登录成功案例则改滑动 |
| L-13 | WS 帧限流（走子 20/10s 等） | 见 §6.6 | 统计真实对局的走子速率分布（P99） |
| L-14 | 幂等键 TTL | 300 s | 实测客户端退避窗口的上界（当前退避上限 30 s） |
| L-15 | 全局请求体上限 | 64 KiB | 实测最长棋谱导入的载荷大小 |

### D 系列 —— 设计待决 / 待实现

| 编号 | 项 | 现状 | 决策方 |
|---|---|---|---|
| D-01 | `users.token_version` 列的引入 | §2.4 已给出结论（需加列），**该列尚未写入 [09 §3.1](09-数据模型与存储设计.md) 的表定义** | 实现前需同步 09 |
| D-02 | `/leaderboard` 周榜的周边界时区 | 暂定 UTC 周一 00:00，**是否改用北京时间待产品确认** | 产品 |
| D-03 | 周榜排序依据（本周净增分 vs 绝对分） | §3.7.1 选"净增分"，理由已给 | 产品确认 |
| D-04 | 举报处理时效承诺 | 文案写"3 个工作日"，**待运营确认** | 运营 |
| D-05 | 大厅列表排序中"观战人数"的可行性 | `spectators_desc` 需要 Redis Set 基数参与排序，**可能成本过高**；⚠️ 待实现时评估 | 实现 |
| D-06 | `GET /games` 的 `OR` 查询改写（`UNION ALL`） | §5.1 已标注必须由 `EXPLAIN ANALYZE` 确认 | 实现 |
| D-07 | 导入棋谱的 `validation_legality=false` 是否保留 | 该选项会放行非法棋谱，**可能成为绕过权威裁决的入口**；⚠️ 需复查是否只用于客户端本地场景 | 安全 |
| D-08 | `Idempotency-Key` 并发在途时的响应码 | §8.2 暂定 `409 SYS008` + `in_flight: true` | 实现 |
| D-09 | `GET /users/{user_id}` 的用户枚举防护 | §6.3 已标注"未在本期实现"，只依赖 IP 限流 | 安全 |
| D-10 | 限流 IP 的取值来源（对端地址 vs `X-Forwarded-For`） | §6.2 标注待部署阶段确认。**取错会让所有用户共享一个桶** | 部署 |
| D-11 | `Deprecation` 最短期 180 天 | §9.2 | 产品 |
| D-12 | WS `MatchJoin` / `MatchLeave` 帧是否标记 deprecated | §7.3 建议标记，**需在 [06-联网协议](06-联网对战与实时通信协议.md) 中落实** | 实现 |
| D-13 | `min_rating` 过滤是否会造成"高手被围观骚扰" | §3.4.1 新增了该参数（服务于学习场景，FR-06.03），**未评估社交影响** | 产品 |
| D-14 | CAPTCHA / 异地登录检测 / 泄露密码库比对 | §6.4 明确**本期不做**，但登记为加固方向 | 产品 / 安全 |
| D-15 | `/metrics` 的暴露方式与端口 | §3.9.3 约定不用业务端口，**具体方案待 M4 部署阶段确定** | 部署 |

### C 系列 —— 与其他文档的接口需求（需回写）

| 编号 | 本文的需求 | 目标文档 |
|---|---|---|
| C-01 | `users.token_version integer NOT NULL DEFAULT 1` 列（§2.4） | [09 §3.1](09-数据模型与存储设计.md) |
| C-02 | WS `Hello` / `HelloAck` 帧的字段定义（`protocol_version` / `rules_version` / `client_version`）与关闭码 `4001` / `4003`（§9.3、§10.1） | [06-联网协议](06-联网对战与实时通信协议.md) |
| C-03 | `MatchFound` 帧需携带 `game_id` / `room_code` / `my_side` / 双方 `rating`（§3.6.1 与 §7.3） | [06-联网协议](06-联网对战与实时通信协议.md) |
| C-04 | `ClockSync` 帧的周期（5 s，[ADR-014](14-决策记录ADR.md#adr-014)）与字段需与 `red_time_left_ms` / `black_time_left_ms` 一致 | [06-联网协议](06-联网对战与实时通信协议.md) |
| C-05 | `xq-protocol` 中所有对外 ID 字段类型必须是 `String`，禁止 `i64`（§1.4）——需有单元测试锁定 | `xq-protocol` |
| C-06 | 错误码常量表 46 项需在 `xq-protocol::ApiError` 中一一实现，且 `retryable` 为**由服务端计算并下发**的字段（§4.1） | `xq-protocol` |
| C-07 | 错误消息不得包含内部细节——需有 CI 断言测试（§4.1 原则 1） | [11-工程规范与CI-CD](11-工程规范与CI-CD.md) |
| C-08 | [07-匹配排行榜与反作弊](07-匹配排行榜与反作弊.md) 的 ELO 参数（K 值、初始分、段位线）需与 `rating_history` 的 `rating_before` / `rating_after` / `delta` 字段语义一致（§3.3.4） | [07](07-匹配排行榜与反作弊.md) |
| C-09 | `estimated_wait_ms` / `queue_length` 的计算口径（§3.6.1）需与匹配模块实现一致 | `xq-server::matchmaking` |
| C-10 | 限流中间件需支持"同一端点多维度叠加"（§6.4 的三重限流）与 `fail-open` 语义（§6.1） | `xq-server` |

