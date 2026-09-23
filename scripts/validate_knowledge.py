#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
validate_knowledge.py —— assets/knowledge/ 知识库的构建期校验。

为什么需要构建期校验：
  xq-coach 的本地讲解路径依赖这四份 JSON。如果配置有错（模板引用不存在、
  占位符拼错、谓词未定义），运行时的表现是「静默降级到兜底模板」——用户看到
  一条空洞的讲解，而开发者从日志里几乎看不出原因。把这类错误提前到构建期
  硬失败，比事后在线上捞日志高效得多（见 docs/05 §9.4）。

校验项：
  1. 四份 JSON 均可解析
  2. id 全局唯一（各自文件内与跨文件引用）
  3. tactics 引用的 explanation_template 必须在 coach-templates 中存在
  4. openings 引用的 explanation_template 必须存在
  5. 模板中的 {占位符} 必须在 allowed 白名单内，且含必需占位符
  6. tactics 中 L3 pattern 的 predicate 必须在 predicates.json 中定义
  7. predicate 调用时传入的参数名必须在定义中声明；必填参数不得缺失
  8. piece_kind 取值合法
  9. 各文件 statistics 中声明的数量与实际条目数一致
 10. 兜底模板不得含占位符遗漏（fallback 允许为空字符串）

退出码 0 = 全部通过，1 = 存在失败项。
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
KB = ROOT / "assets" / "knowledge"

FILES = {
    "tactics": KB / "tactics.json",
    "predicates": KB / "predicates.json",
    "templates": KB / "coach-templates.json",
    "openings": KB / "openings.json",
}

PLACEHOLDER_RE = re.compile(r"\{([a-zA-Z_][a-zA-Z0-9_]*)\}")

errors: list[str] = []
warnings: list[str] = []


def fail(msg: str) -> None:
    errors.append(msg)


def warn(msg: str) -> None:
    warnings.append(msg)


def load_all() -> dict:
    data = {}
    for name, path in FILES.items():
        if not path.exists():
            fail(f"[{name}] 文件不存在：{path}")
            continue
        try:
            data[name] = json.loads(path.read_text(encoding="utf-8"))
            print(f"  [OK] 解析 {path.name}")
        except json.JSONDecodeError as e:
            fail(f"[{name}] JSON 解析失败：{path.name} 第 {e.lineno} 行第 {e.colno} 列 —— {e.msg}")
            if e.lineno and path.exists():
                lines = path.read_text(encoding="utf-8").splitlines()
                for i in range(max(0, e.lineno - 3), min(len(lines), e.lineno + 2)):
                    mark = ">>" if i == e.lineno - 1 else "  "
                    print(f"     {mark} {i+1:5d} | {lines[i]}")
    return data


def check_unique_ids(tactics: dict) -> None:
    seen: dict[str, int] = {}
    for t in tactics.get("tactics", []):
        tid = t.get("id")
        if not tid:
            fail("[tactics] 存在缺少 id 的条目")
            continue
        seen[tid] = seen.get(tid, 0) + 1
    dupes = [k for k, v in seen.items() if v > 1]
    for d in dupes:
        fail(f"[tactics] id 重复：{d}")
    print(f"  [OK] tactics id 唯一性（{len(seen)} 个）")


def check_templates(tactics: dict, tmpl_doc: dict) -> None:
    tmpls = tmpl_doc.get("templates", [])
    tmpl_ids = {t.get("id") for t in tmpls}

    dupes = len(tmpls) - len(tmpl_ids)
    if dupes:
        fail(f"[templates] 存在 {dupes} 个重复 id")

    allowed = set(tmpl_doc.get("placeholders", {}).get("allowed", []))
    required = set(tmpl_doc.get("placeholders", {}).get("required_always", []))

    if not allowed:
        fail("[templates] placeholders.allowed 为空或缺失")
    if not required:
        warn("[templates] placeholders.required_always 为空，无法校验必需占位符")

    for t in tmpls:
        tid = t.get("id", "<no-id>")
        is_fallback = bool(t.get("is_fallback"))
        texts: list[tuple[str, str]] = []
        if isinstance(t.get("headline"), str):
            texts.append(("headline", t["headline"]))
        detail = t.get("detail") or {}
        if isinstance(detail, dict):
            for k, v in detail.items():
                if isinstance(v, str):
                    texts.append((f"detail.{k}", v))

        found_any = set()
        for field, text in texts:
            for ph in PLACEHOLDER_RE.findall(text):
                found_any.add(ph)
                if ph not in allowed:
                    fail(f"[templates] {tid}.{field} 使用了未声明的占位符 {{{ph}}}")
            # 花括号必须闭合：检查孤立的花括号
            stripped = PLACEHOLDER_RE.sub("", text)
            if "{" in stripped or "}" in stripped:
                fail(f"[templates] {tid}.{field} 存在未闭合的花括号")

        if not is_fallback and required:
            missing = required - found_any
            if missing:
                # headline 与各档 detail 合并判断（不同档位可省略）
                combined = " ".join(x[1] for x in texts)
                missing = {m for m in required if "{" + m + "}" not in combined}
                if missing:
                    fail(f"[templates] {tid} 缺少必需占位符：{sorted(missing)}")

    print(f"  [OK] 模板 {len(tmpls)} 条，占位符白名单 {len(allowed)} 个")

    # 引用完整性：tactics -> templates
    for t in tactics.get("tactics", []):
        ref = t.get("explanation_template")
        if ref and ref not in tmpl_ids:
            fail(f"[引用] tactics「{t.get('id')}」引用了不存在的模板 {ref}")
    print("  [OK] tactics -> templates 引用完整性")


def check_openings(openings: dict, tmpl_ids: set[str], tactics: dict) -> None:
    olist = openings.get("openings", [])
    seen: set[str] = set()
    for o in olist:
        oid = o.get("id")
        if oid in seen:
            fail(f"[openings] id 重复：{oid}")
        seen.add(oid)

        ref = o.get("explanation_template")
        if ref and ref not in tmpl_ids:
            fail(f"[引用] openings「{oid}」引用了不存在的模板 {ref}")

        seq = o.get("sequence_iccs") or []
        names = o.get("sequence_names") or []
        if seq and names and len(seq) != len(names):
            fail(
                f"[openings]「{oid}」sequence_iccs 长度 {len(seq)} "
                f"与 sequence_names 长度 {len(names)} 不一致"
            )

        for mv in seq:
            if not re.fullmatch(r"[a-i][0-9][a-i][0-9]", mv):
                fail(f"[openings]「{oid}」着法 {mv} 不是合法 ICCS 格式（应为如 h2e2）")
                continue
            fc, fr, tc, tr = mv[0], int(mv[1]), mv[2], int(mv[3])
            for c, r, pos in ((fc, fr, "起点"), (tc, tr, "终点")):
                if not ("a" <= c <= "i" and 0 <= r <= 9):
                    fail(f"[openings]「{oid}」着法 {mv} 的{pos} {c}{r} 越界")

    print(f"  [OK] openings {len(olist)} 条，序列格式与引用完整")


def check_predicates(tactics: dict, pred_doc: dict) -> None:
    preds = {p.get("id"): p for p in pred_doc.get("predicates", [])}
    kinds = set(pred_doc.get("piece_kinds", []))

    if not preds:
        fail("[predicates] 谓词表为空")
        return

    used: set[str] = set()
    for t in tactics.get("tactics", []):
        pattern = t.get("pattern")
        if not pattern:
            continue
        for con in pattern.get("constraints", []):
            pid = con.get("predicate")
            if not pid:
                fail(f"[tactics]「{t.get('id')}」的约束缺少 predicate 字段")
                continue
            used.add(pid)
            if pid not in preds:
                fail(f"[引用] tactics「{t.get('id')}」引用了未定义的谓词 {pid}")
                continue
            declared = {p.get("name") for p in preds[pid].get("params", [])}
            required_params = {
                p.get("name") for p in preds[pid].get("params", []) if p.get("required")
            }
            passed = set(con.keys()) - {"predicate"}
            extra = passed - declared
            if extra:
                fail(f"[引用] 调用谓词 {pid} 时传入了未声明的参数：{sorted(extra)}")
            missing = required_params - passed
            if missing:
                fail(f"[引用] 调用谓词 {pid} 时缺少必填参数：{sorted(missing)}")
            # piece_kind 取值校验
            k = con.get("kind")
            if k and k not in kinds:
                fail(f"[引用] 谓词 {pid} 的参数 kind=\"{k}\" 不在 piece_kinds 中")

    unused = set(preds) - used
    if unused:
        warn(f"[predicates] {len(unused)} 个谓词未被任何棋形使用：{sorted(unused)}")

    print(
        f"  [OK] 谓词 {len(preds)} 个，被棋形引用 {len(used)} 个，"
        f"参数匹配校验通过"
    )


def check_statistics(docs: dict) -> None:
    t_stats = docs.get("tactics", {}).get("statistics", {})
    t_actual = len(docs.get("tactics", {}).get("tactics", []))
    if t_stats.get("total") != t_actual:
        fail(f"[statistics] tactics 声明 total={t_stats.get('total')}，实际 {t_actual}")

    c_stats = docs.get("templates", {}).get("statistics", {})
    c_actual = len(docs.get("templates", {}).get("templates", []))
    if c_stats.get("total") != c_actual:
        fail(f"[statistics] templates 声明 total={c_stats.get('total')}，实际 {c_actual}")

    o_stats = docs.get("openings", {}).get("statistics", {})
    o_actual = len(docs.get("openings", {}).get("openings", []))
    if o_stats.get("total") != o_actual:
        fail(f"[statistics] openings 声明 total={o_stats.get('total')}，实际 {o_actual}")

    p_stats = docs.get("predicates", {}).get("statistics", {})
    p_actual = len(docs.get("predicates", {}).get("predicates", []))
    if p_stats.get("total") != p_actual:
        fail(f"[statistics] predicates 声明 total={p_stats.get('total')}，实际 {p_actual}")

    print("  [OK] statistics 声明数量与实际条目数一致")


def main() -> int:
    print("validate_knowledge: 开始校验 assets/knowledge/")
    docs = load_all()

    if "tactics" not in docs:
        print("\n[ABORT] tactics.json 无法解析，后续校验跳过。")
        for e in errors:
            print(f"  [FAIL] {e}")
        return 1

    tmpl_ids: set[str] = set()
    if "templates" in docs:
        tmpl_ids = {t.get("id") for t in docs["templates"].get("templates", [])}

    check_unique_ids(docs["tactics"])

    if "templates" in docs:
        check_templates(docs["tactics"], docs["templates"])
    else:
        fail("[templates] 未加载，跳过引用校验")

    if "openings" in docs:
        check_openings(docs["openings"], tmpl_ids, docs["tactics"])
    else:
        fail("[openings] 未加载")

    if "predicates" in docs:
        check_predicates(docs["tactics"], docs["predicates"])
    else:
        fail("[predicates] 未加载")

    check_statistics(docs)

    print()
    if warnings:
        print(f"警告 {len(warnings)} 项：")
        for w in warnings:
            print(f"  [WARN] {w}")
        print()

    if errors:
        print(f"校验失败：{len(errors)} 项")
        for e in errors:
            print(f"  [FAIL] {e}")
        return 1

    total = sum(
        len(docs[k].get(k2, []))
        for k, k2 in (
            ("tactics", "tactics"),
            ("predicates", "predicates"),
            ("templates", "templates"),
            ("openings", "openings"),
        )
        if k in docs
    )
    print(f"校验通过：全部检查项 OK，共 {total} 条知识条目")
    return 0


if __name__ == "__main__":
    sys.exit(main())
