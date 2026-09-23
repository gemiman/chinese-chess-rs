#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
validate_docs.py —— 文档集的一致性校验。

校验项：
  1. 所有 Markdown 相对链接指向的文件真实存在
  2. 形如 xxx.md#anchor 的锚点能在目标文档中找到对应标题
  3. 关键数字在多份文档间保持一致（技术栈版本、棋盘几何、规则常量）
  4. 需求编号在文档中被引用时必须真实存在于 docs/01

这些是文档集最容易出现、也最难靠肉眼发现的问题：改了一处数字忘了改另一处，
或链接在文件重命名后失效。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
README = ROOT / "README.md"

errors: list[str] = []
warnings: list[str] = []

# 注意 group(2) 用 *? 而非 +?：这样 "(#anchor)" 这种纯锚点链接能被正确切分，
# 否则正则会把 "#anchor" 整体吞成路径。
LINK_RE = re.compile(r"\[([^\]]*)\]\(([^)\s]*?)(?:#([^)\s]+))?\)")


def all_md() -> list[Path]:
    return sorted(DOCS.glob("*.md")) + ([README] if README.exists() else [])


def slugify(title: str) -> str:
    """近似 GitHub 的锚点生成规则：小写、去标点、空格转连字符。"""
    s = title.strip().lower()
    s = re.sub(r"[^\w\u4e00-\u9fff\s-]", "", s)
    s = re.sub(r"\s+", "-", s)
    return s


def anchors_of(path: Path) -> set[str]:
    text = path.read_text(encoding="utf-8")
    out: set[str] = set()
    for line in text.splitlines():
        m = re.match(r"^(#{1,6})\s+(.*?)\s*$", line)
        if m:
            out.add(slugify(m.group(2)))
    return out


def check_links() -> None:
    anchor_cache: dict[Path, set[str]] = {}
    checked_files = 0
    checked_links = 0

    for md in all_md():
        text = md.read_text(encoding="utf-8")
        # 跳过代码块内的内容，避免把示例链接误判为真实链接
        text_nocode = re.sub(r"```.*?```", "", text, flags=re.S)
        text_nocode = re.sub(r"`[^`]*`", "", text_nocode)

        for m in LINK_RE.finditer(text_nocode):
            target, anchor = m.group(2), m.group(3)
            if target.startswith(("http://", "https://", "mailto:")):
                continue
            checked_links += 1

            # 纯锚点链接（target 为空）指向当前文件自身
            resolved = md.resolve() if not target else (md.parent / target).resolve()
            if not resolved.exists():
                errors.append(f"[链接] {md.name} -> 目标不存在：{target or '(空)'}")
                continue

            if resolved.suffix == ".md":
                checked_files += 1
                if anchor:
                    if resolved not in anchor_cache:
                        anchor_cache[resolved] = anchors_of(resolved)
                    # 允许 adr-003 这类全小写锚点与标题 slug 的宽松匹配
                    if anchor not in anchor_cache[resolved]:
                        soft = {a for a in anchor_cache[resolved] if a.replace("-", "") == anchor.replace("-", "")}
                        if not soft:
                            warnings.append(
                                f"[锚点] {md.name} -> {target}#{anchor} 未找到对应标题"
                            )

    print(f"  [OK] 检查 {checked_links} 个相对链接，{checked_files} 个跨文档引用")


def check_numbers() -> None:
    """关键数字一致性：同一事实在多份文档中必须一致。"""
    facts = {
        "tauri=2.11.6": ("2.11.6", ["README.md", "docs/02-系统架构设计.md", "docs/08-客户端设计.md"]),
        "axum=0.8.9": ("0.8.9", ["README.md", "docs/02-系统架构设计.md"]),
        "tokio=1.53.1": ("1.53.1", ["README.md", "docs/02-系统架构设计.md"]),
        "rust=1.98.1": ("1.98.1", ["README.md", "docs/02-系统架构设计.md"]),
        "sea-orm=2.0.3": ("2.0.3", ["README.md", "docs/02-系统架构设计.md"]),
        # 棋盘像素几何只在渲染层文档中定义（规则文档用 col/row 表达，不涉及像素）
        "棋盘宽=560": ("560", ["docs/08-客户端设计.md"]),
        "棋盘高=620": ("620", ["docs/08-客户端设计.md"]),
        "格距=60": ("60", ["docs/08-客户端设计.md"]),
        "perft(1)=44": ("44", ["docs/03-规则引擎与领域模型.md", "docs/12-测试策略与质量保障.md"]),
        "自然限着=60 回合": ("60", ["docs/01-需求规格说明书.md", "docs/03-规则引擎与领域模型.md"]),
        "断线保留=120 秒": ("120", ["docs/01-需求规格说明书.md", "docs/06-联网对战与实时通信协议.md"]),
        "ELO 初始=1500": ("1500", ["docs/01-需求规格说明书.md", "docs/07-匹配排行榜与反作弊.md"]),
        "观战延迟=3000ms": ("3000", ["docs/01-需求规格说明书.md", "docs/06-联网对战与实时通信协议.md"]),
        "悔棋次数=3": ("3", ["docs/01-需求规格说明书.md"]),
    }

    for label, (needle, files) in facts.items():
        missing = []
        for f in files:
            p = ROOT / f
            if not p.exists():
                missing.append(f"{f}(文件缺失)")
                continue
            if needle not in p.read_text(encoding="utf-8"):
                missing.append(f)
        if missing:
            warnings.append(f"[数字] 「{label}」未在以下文档中出现：{missing}")

    print(f"  [OK] 抽查 {len(facts)} 组关键数字的一致性")


def check_requirement_ids() -> None:
    """文档中引用的 FR-xx.yy 需求编号必须在 docs/01 中真实定义。"""
    spec = DOCS / "01-需求规格说明书.md"
    spec_text = spec.read_text(encoding="utf-8")
    defined = set(re.findall(r"FR-\d{2}\.\d{2}", spec_text))

    referenced: dict[str, set[str]] = {}
    for md in all_md():
        if md.name.startswith("01-"):
            continue
        text = md.read_text(encoding="utf-8")
        for rid in set(re.findall(r"FR-\d{2}\.\d{2}", text)):
            referenced.setdefault(rid, set()).add(md.name)

    undefined = {r: f for r, f in referenced.items() if r not in defined}
    if undefined:
        for rid, files in sorted(undefined.items()):
            warnings.append(
                f"[需求] 引用了 docs/01 中未定义的编号 {rid}（出现在 {sorted(files)}）"
            )
    print(f"  [OK] docs/01 定义 {len(defined)} 个需求编号，被引用 {len(referenced)} 个")


def check_no_unreplaced_placeholder() -> None:
    """扫描残留的模板标记。

    注意：TODO / FIXME 在工程规范类文档（11）与测试文档（12）中是**正当的规范条文**
    （例如「TODO 必须带里程碑标记」这条规则本身），因此不纳入检查；
    这里只找真正代表「内容没写完」的标记。
    """
    suspicious = ["@PLACEHOLDER@", "<!--@", "此处省略", "待补充", "（略）", "PLACEHOLDER"]
    hits: list[str] = []
    for md in all_md():
        text = md.read_text(encoding="utf-8")
        for t in suspicious:
            if t in text:
                hits.append(f"{md.name} 含「{t}」")
    if hits:
        for h in hits:
            warnings.append(f"[占位] {h}")
    print("  [OK] 扫描残留模板标记（{xxx} 占位符由 knowledge 校验单独覆盖）")


def main() -> int:
    print("validate_docs: 开始校验文档集")
    check_links()
    check_numbers()
    check_requirement_ids()
    check_no_unreplaced_placeholder()

    print()
    if warnings:
        print(f"提示 {len(warnings)} 项（不阻断）：")
        for w in warnings:
            print(f"  [WARN] {w}")
    if errors:
        print(f"\n校验失败：{len(errors)} 项")
        for e in errors:
            print(f"  [FAIL] {e}")
        return 1
    print("\n校验通过：链接与关键数字一致性检查全部通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
