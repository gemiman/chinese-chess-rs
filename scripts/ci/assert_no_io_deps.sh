#!/usr/bin/env bash
# ============================================================
# ADR-005 依赖断言
#
# 校验领域层 crate（xq-core / xq-ai / xq-coach）的依赖树中不出现任何
# IO / 运行时 / 存储 crate，并且源码中不出现被禁的 std 用法。
#
# 这是不变量 I-1「领域层零 IO」的唯一强制执行点。没有它，零 IO 约束
# 会在几个月内被无声破坏。
#
# 用法： bash scripts/ci/assert_no_io_deps.sh
# 退出码：0 = 通过；1 = 违反约束
#
# 参见：docs/11 §5.3 · docs/02 §2.1 不变量 I-1 · docs/14 ADR-005
# ============================================================
set -euo pipefail

cd "$(dirname "$0")/../.." || exit 1

# 被禁依赖（ADR-005 明确列举）
BANNED_NORMAL=(
  tokio axum sqlx sea-orm sea-orm-migration
  redis deadpool-redis reqwest hyper
)

# 领域层三个 crate（尚未创建的会被自动跳过）
DOMAIN_CRATES=(xq-core xq-ai xq-coach)

# 领域层禁止出现在源码中的 std 用法（ADR-005）
BANNED_SOURCE_PATTERNS=(
  'std::fs'
  'std::net'
  'std::time::SystemTime'
  'std::process::'
  'thread_rng'
  'rand::random'
)

# Cargo.lock 存在时使用 --locked，保证断言的是锁定后的依赖树
LOCKED_FLAG=()
[ -f Cargo.lock ] && LOCKED_FLAG=(--locked)

FAILED=0

log()  { printf '%s\n' "$*"; }
fail() { printf '\033[31m%s\033[0m\n' "$*"; FAILED=1; }
ok()   { printf '\033[32m%s\033[0m\n' "$*"; }

# ------------------------------------------------------------
# 0. 筛选出实际存在的领域层 crate
# ------------------------------------------------------------
PRESENT_CRATES=()
for crate in "${DOMAIN_CRATES[@]}"; do
  if [ -f "crates/${crate}/Cargo.toml" ]; then
    PRESENT_CRATES+=("${crate}")
  else
    log "==> 跳过 ${crate}（尚未创建）"
  fi
done

if [ ${#PRESENT_CRATES[@]} -eq 0 ]; then
  fail "未找到任何领域层 crate，断言无法执行"
  exit 1
fi

# ------------------------------------------------------------
# 1. cargo tree —— 生产依赖与构建依赖中不得出现被禁 crate
# ------------------------------------------------------------
for crate in "${PRESENT_CRATES[@]}"; do
  log "==> [cargo tree] ${crate}（edges: normal,build）"

  # --prefix none      每个包一行，便于精确 grep
  # --edges normal,build  只查生产与构建依赖
  tree="$(cargo tree -p "${crate}" --edges normal,build --prefix none "${LOCKED_FLAG[@]}")"

  for banned in "${BANNED_NORMAL[@]}"; do
    # 匹配形如 "tokio v1.53.1" 的整行，避免误伤 "tokio-util" 之类
    if printf '%s\n' "${tree}" | grep -Eq "^${banned} v[0-9]"; then
      fail "  [违反 ADR-005] ${crate} 依赖了 ${banned}"
      printf '%s\n' "${tree}" | grep -E "^${banned} v[0-9]" | sed 's/^/      /'
    fi
  done

  # 提示性检查：C 语言依赖会破坏「跨平台纯 Rust 构建」
  if printf '%s\n' "${tree}" | grep -Eq '^openssl-sys v'; then
    log "  [提示] ${crate} 依赖了 openssl-sys，请确认是否有纯 Rust 替代"
  fi

  # 信息：生产依赖条数（不含 crate 自身那一行）
  dep_count=$(( $(printf '%s\n' "${tree}" | grep -c .) - 1 ))
  log "  [信息] ${crate} 生产+构建依赖共 ${dep_count} 个"
  if [ "${dep_count}" -eq 0 ]; then
    ok "  ${crate} 零运行时依赖 ✔"
  else
    ok "  ${crate} 依赖树未含被禁 crate"
  fi
done

# ------------------------------------------------------------
# 2. dev-dependencies 中的被禁 crate —— 仅告警，不失败
#    理由：领域层测试允许使用 tokio 做并发压力测试，但应尽量避免；
#         默认告警可让评审注意到这个信号。
# ------------------------------------------------------------
for crate in "${PRESENT_CRATES[@]}"; do
  dev_tree="$(cargo tree -p "${crate}" --edges dev --prefix none "${LOCKED_FLAG[@]}" 2>/dev/null || true)"
  for banned in "${BANNED_NORMAL[@]}"; do
    if printf '%s\n' "${dev_tree}" | grep -Eq "^${banned} v[0-9]"; then
      log "  [告警] ${crate} 的 dev-dependencies 含 ${banned}（允许但需评审）"
    fi
  done
done

# ------------------------------------------------------------
# 3. 源码级断言 —— 领域层不得直接使用 IO / 系统时钟 / 线程随机
# ------------------------------------------------------------
for crate in "${PRESENT_CRATES[@]}"; do
  src_dir="crates/${crate}/src"
  [ -d "${src_dir}" ] || { log "==> 跳过 ${crate}（src 目录不存在）"; continue; }

  log "==> [源码断言] ${src_dir}"

  for pattern in "${BANNED_SOURCE_PATTERNS[@]}"; do
    # 先剔除注释行（// 与 /// 与 //!），避免文档里举例说明时被误报
    hits="$(
      grep -rn --include='*.rs' -F "${pattern}" "${src_dir}" \
        | grep -vE ':[0-9]+:[[:space:]]*//' \
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
# 4. 错误消息红线（docs/02 §10 原则 1、docs/10 C-07）
#    对外错误不得拼接内部细节
# ------------------------------------------------------------
log "==> [错误消息红线] 对外错误不得拼接内部细节"

for file in crates/xq-server/src/error.rs crates/xq-protocol/src/error.rs; do
  [ -f "${file}" ] || continue
  if grep -nE 'format!\("[^"]*\{:\?\}' "${file}" | grep -vE ':[0-9]+:[[:space:]]*//' >/dev/null 2>&1; then
    fail "  [违反 docs/02 §10] ${file} 中对外错误消息使用了 {e:?} 调试格式"
  fi
done

# ------------------------------------------------------------
# 汇总
# ------------------------------------------------------------
printf '\n%s\n' "------------------------------------------------------------"
if [ "${FAILED}" -ne 0 ]; then
  fail "ADR-005 依赖断言失败：存在违反零 IO 约束的项"
  exit 1
fi
ok "ADR-005 依赖断言通过：领域层零 IO 约束成立"
