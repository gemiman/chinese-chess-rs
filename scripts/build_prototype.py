#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
build_prototype.py —— 由真实素材生成自包含的交互原型 HTML。

为什么用脚本而不是手写 HTML：
  棋盘与棋子必须与 assets/ 下的实现基线**逐字节一致**。手工抄写 SVG 会引入
  抄写误差（坐标偏移、配色笔误），而这类误差在原型评审阶段极难被发现。
  本脚本把素材原样内联，并在生成时用断言校验几何，确保原型可信。

输入：
  assets/board/board-classic.svg
  assets/pieces/{red,black}_{king,general,advisor,elephant,horse,chariot,cannon,pawn}.svg
  prototype/_template.html
输出：
  prototype/index.html

幂等：重复运行结果一致。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
TEMPLATE = ROOT / "prototype" / "_template.html"
OUTPUT = ROOT / "prototype" / "index.html"

# ---------------------------------------------------------------- 几何常量
# 与 assets/tokens/design-tokens.json 的 board 组保持一致
VIEW_W, VIEW_H = 560, 620
MARGIN, CELL = 40, 60
COLS, ROWS = 9, 10
PIECE_DRAW = 64            # 棋子素材 viewBox 边长
PIECE_ON_BOARD = 52        # 棋子落在棋盘上的直径（cell * 0.87 = 52.2 → 取 52）
SCALE = PIECE_ON_BOARD / PIECE_DRAW
HALF = PIECE_ON_BOARD / 2

# 14 种棋子素材（红 7 + 黑 7）
PIECE_FILES = [
    "red_king", "red_advisor", "red_elephant", "red_horse",
    "red_chariot", "red_cannon", "red_pawn",
    "black_general", "black_advisor", "black_elephant", "black_horse",
    "black_chariot", "black_cannon", "black_pawn",
]

# ------------------------------------------------- 演示局面（合法且自洽）
# 走法序列（ICCS，红先）：
#   1 红 h2e2 炮二平五     2 黑 h9g7 马8进7
#   3 红 h0g2 马二进三     4 黑 i9h9 车9平8
#   5 红 i0h0 车一平二     6 黑 g6g5 卒7进1
#   7 红 h0h6 车二进六     8 黑 b9c7 马2进3
#   9 红 b0c2 马八进七    10 黑 a9b9 车1平2
# 走完第 10 着后轮红方，当前选中 h6 红车（可走 8 个空位 + 可吃 3 子）。
# 该局面的 FEN 由本脚本按同一份数据渲染，原型与文档描述严格对应。
POSITION = [
    # (素材名, col, row, 备注：该格上的棋子)
    # ---- 黑方 ----
    ("black_chariot",  1, 9, "车1平2 后 位于 b9"),
    ("black_chariot",  7, 9, "车9平8 后 位于 h9"),
    ("black_elephant", 2, 9, "象 c9"),
    ("black_advisor",  3, 9, "士 d9"),
    ("black_general",  4, 9, "将 e9"),
    ("black_advisor",  5, 9, "士 f9"),
    ("black_elephant", 6, 9, "象 g9"),
    ("black_horse",    2, 7, "马2进3 后 位于 c7"),
    ("black_horse",    6, 7, "马8进7 后 位于 g7"),
    ("black_cannon",   1, 7, "炮 b7"),
    ("black_cannon",   7, 7, "炮 h7"),
    ("black_pawn",     0, 6, "卒 a6"),
    ("black_pawn",     2, 6, "卒 c6"),
    ("black_pawn",     4, 6, "卒 e6"),
    ("black_pawn",     6, 5, "卒7进1 后 位于 g5"),
    ("black_pawn",     8, 6, "卒 i6"),
    # ---- 红方 ----
    ("red_chariot",    0, 0, "车 a0"),
    ("red_elephant",   2, 0, "相 c0"),
    ("red_advisor",    3, 0, "仕 d0"),
    ("red_king",       4, 0, "帅 e0"),
    ("red_advisor",    5, 0, "仕 f0"),
    ("red_elephant",   6, 0, "相 g0"),
    ("red_cannon",     1, 2, "炮 b2"),
    ("red_cannon",     4, 2, "炮二平五 后 位于 e2"),
    ("red_horse",      2, 2, "马八进七 后 位于 c2"),
    ("red_horse",      6, 2, "马二进三 后 位于 g2"),
    ("red_pawn",       0, 3, "兵 a3"),
    ("red_pawn",       2, 3, "兵 c3"),
    ("red_pawn",       4, 3, "兵 e3"),
    ("red_pawn",       6, 3, "兵 g3"),
    ("red_pawn",       8, 3, "兵 i3"),
    ("red_chariot",    7, 6, "车二进六 后 位于 h6（当前选中）"),
]

EXPECTED_COUNTS = {
    "black_chariot": 2, "black_horse": 2, "black_cannon": 2, "black_pawn": 5,
    "black_elephant": 2, "black_advisor": 2, "black_general": 1,
    "red_chariot": 2, "red_horse": 2, "red_cannon": 2, "red_pawn": 5,
    "red_elephant": 2, "red_advisor": 2, "red_king": 1,
}


def read_utf8(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def svg_inner(path: Path) -> str:
    """取出 <svg> 标签内部的内容，defs 一并保留。

    棋盘的浮雕滤镜与棋子的渐变/投影都在 defs 里，剥掉会让立体效果全部失效；
    各类 id 已按棋子名 / board- 前缀区分，内联到同一文档不会冲突。
    """
    text = read_utf8(path)
    m = re.search(r"<svg\b[^>]*>(.*)</svg>", text, re.S)
    if not m:
        raise SystemExit(f"[FAIL] 无法解析 SVG：{path}")
    return m.group(1).strip()


def build_piece_sprites() -> str:
    parts = []
    for name in PIECE_FILES:
        path = ASSETS / "pieces" / f"{name}.svg"
        if not path.exists():
            raise SystemExit(f"[FAIL] 缺少棋子素材：{path}")
        inner = svg_inner(path)
        parts.append(f'    <g id="pc-{name}">{inner}</g>')
    return "\n".join(parts)


def build_piece_layer() -> str:
    # 先按 (row, col) 排序，保证输出稳定（幂等的前提之一）
    ordered = sorted(POSITION, key=lambda t: (t[2], t[1]))
    lines = []
    for name, col, row, note in ordered:
        cx = MARGIN + col * CELL
        # 关键：SVG 的 y 轴向下，而棋理的行号 0 是红方底线（视觉下方）。
        # 因此必须做垂直翻转，否则红黑双方会上下颠倒。
        cy = MARGIN + (ROWS - 1 - row) * CELL
        tx = round(cx - HALF, 2)
        ty = round(cy - HALF, 2)
        tx = int(tx) if float(tx).is_integer() else tx
        ty = int(ty) if float(ty).is_integer() else ty
        lines.append(
            f'              <use href="#pc-{name}" '
            f'transform="translate({tx},{ty}) scale({SCALE:.4f})"/>'
            f"   <!-- {note} -->"
        )
    return "\n".join(lines)


def assert_geometry(board_svg: str) -> None:
    """对素材几何做断言，避免原型悄悄偏离设计基线。"""
    inner = svg_inner(ASSETS / "board" / "board-classic.svg")

    h_lines = len(re.findall(r'<line[^>]*class="grid-line"[^>]*x1="40"[^>]*y1="(\d+)"[^>]*x2="520"',
                             inner))
    # 更稳妥：直接按坐标特征统计
    all_lines = re.findall(r'<line\b[^>]*/>', inner)
    horizontal, vertical_segments = 0, 0
    for ln in all_lines:
        x1 = re.search(r'x1="([\d.]+)"', ln)
        x2 = re.search(r'x2="([\d.]+)"', ln)
        y1 = re.search(r'y1="([\d.]+)"', ln)
        y2 = re.search(r'y2="([\d.]+)"', ln)
        if not (x1 and x2 and y1 and y2):
            continue
        x1, x2, y1, y2 = (float(g.group(1)) for g in (x1, x2, y1, y2))
        if y1 == y2:
            horizontal += 1
        elif x1 == x2:
            vertical_segments += 1

    # 九宫斜线（4 条）与兵炮位标记（折角 polyline）
    diagonals = 0
    for ln in all_lines:
        x1 = re.search(r'x1="([\d.]+)"', ln)
        x2 = re.search(r'x2="([\d.]+)"', ln)
        y1 = re.search(r'y1="([\d.]+)"', ln)
        y2 = re.search(r'y2="([\d.]+)"', ln)
        if x1 and x2 and y1 and y2:
            x1, x2, y1, y2 = (float(g.group(1)) for g in (x1, x2, y1, y2))
            if x1 != x2 and y1 != y2:
                diagonals += 1
    horizontals_excl_diag = horizontal  # 斜线的 y1 != y2，故不计入

    polylines = len(re.findall(r"<polyline\b", inner))

    checks = [
        ("横线 10 条", horizontals_excl_diag, 10),
        ("竖线 16 段", vertical_segments, 16),
        ("九宫斜线 4 条", diagonals, 4),
    ]
    for label, got, want in checks:
        if got != want:
            raise SystemExit(f"[FAIL] 棋盘几何断言失败：{label} 期望 {want}，实际 {got}")
        print(f"  [OK] {label}")

    # 折角：兵位 10 点 × 4 折角 = 40，炮位 4 点 × 4 折角 = 16；边线点各减 2 → 共 48
    if polylines != 48:
        raise SystemExit(f"[FAIL] 兵炮位折角断言失败：期望 48，实际 {polylines}")
    print(f"  [OK] 兵炮位折角 48 个（polyline）")

    if 'viewBox="0 0 560 620"' not in read_utf8(ASSETS / "board" / "board-classic.svg"):
        raise SystemExit("[FAIL] 棋盘 viewBox 不是 0 0 560 620")
    print("  [OK] 棋盘 viewBox 0 0 560 620")

    cols_present = {c for _, c, _, _ in POSITION}
    rows_present = {r for _, _, r, _ in POSITION}
    if not cols_present <= set(range(COLS)) or not rows_present <= set(range(ROWS)):
        raise SystemExit("[FAIL] 演示局面存在越界坐标")


def assert_pieces() -> None:
    counts: dict[str, int] = {}
    for name, _, _, _ in POSITION:
        counts[name] = counts.get(name, 0) + 1
    for name, want in EXPECTED_COUNTS.items():
        got = counts.get(name, 0)
        if got != want:
            raise SystemExit(f"[FAIL] 棋子数量断言失败：{name} 期望 {want}，实际 {got}")
    total = sum(counts.values())
    if total != 32:
        raise SystemExit(f"[FAIL] 棋子总数期望 32，实际 {total}")
    print(f"  [OK] 棋子 32 枚，14 种类型数量与标准开局一致")
    if len(PIECE_FILES) != 14:
        raise SystemExit("[FAIL] 棋子素材应为 14 个")


def assert_highlight_alignment(template: str) -> None:
    """校验模板高亮层坐标都落在棋盘交叉点上，且选中环与演示局面的 h6 红车重合。

    高亮坐标是硬编码在模板里的（便于人工审阅），因此必须在生成时与棋子坐标
    做一次机器校验——否则调整演示局面时极易漏改高亮层，产生"高亮偏移一格"
    这类在评审时很难被发现的缺陷。
    """
    valid_x = {MARGIN + c * CELL for c in range(COLS)}
    valid_y = {MARGIN + r * CELL for r in range(ROWS)}

    m = re.search(r'<g id="layer-highlight">(.*?)</g>', template, re.S)
    if not m:
        raise SystemExit("[FAIL] 模板缺少 layer-highlight 分组")
    seg = m.group(1)

    pts = re.findall(r'cx="([\d.]+)"\s+cy="([\d.]+)"', seg)
    if not pts:
        raise SystemExit("[FAIL] 高亮层未找到任何圆点坐标")
    for x, y in pts:
        if float(x) not in valid_x or float(y) not in valid_y:
            raise SystemExit(f"[FAIL] 高亮圆点 ({x},{y}) 不在棋盘交叉点上")
    print(f"  [OK] 高亮层 {len(pts)} 个圆点坐标全部落在交叉点上")

    # 选中环必须与 h6（col 7, row 6）的红车重合
    expect_cx = MARGIN + 7 * CELL
    expect_cy = MARGIN + (ROWS - 1 - 6) * CELL
    m2 = re.search(r'class="hl-sel-ring"\s+cx="([\d.]+)"\s+cy="([\d.]+)"', seg)
    if not m2:
        raise SystemExit("[FAIL] 模板缺少选中环 hl-sel-ring")
    if float(m2.group(1)) != expect_cx or float(m2.group(2)) != expect_cy:
        raise SystemExit(
            f"[FAIL] 选中环 ({m2.group(1)},{m2.group(2)}) 与 h6 红车 "
            f"({expect_cx},{expect_cy}) 不重合"
        )
    print(f"  [OK] 选中环与 h6 红车 ({expect_cx},{expect_cy}) 重合")

    rects = re.findall(r'<rect class="hl-last-\w+"\s+x="([\d.]+)"\s+y="([\d.]+)"', seg)
    for x, y in rects:
        if float(x) + 12 not in valid_x or float(y) + 12 not in valid_y:
            raise SystemExit(
                f"[FAIL] 上一着方框中心 ({float(x)+12},{float(y)+12}) 不在交叉点上"
            )
    print(f"  [OK] 上一着方框 {len(rects)} 个坐标合规")


def main() -> int:
    print("build_prototype: 开始")

    for p in (TEMPLATE, ASSETS / "board" / "board-classic.svg"):
        if not p.exists():
            raise SystemExit(f"[FAIL] 缺少输入文件：{p}")

    # ---- 校验 ----
    print(" 校验素材几何：")
    board_inner = svg_inner(ASSETS / "board" / "board-classic.svg")
    assert_geometry(board_inner)
    assert_pieces()
    print(" 校验高亮层与局面的一致性：")
    assert_highlight_alignment(read_utf8(TEMPLATE))

    # ---- 组装 ----
    sprites = build_piece_sprites()
    pieces_layer = build_piece_layer()

    # 复盘页需要第二份棋盘实例：给全部 board-* 的 id 加后缀，避免同一文档内 id 冲突
    board_inner_2 = re.sub(r'id="(board-[a-z-]+)"', r'id="\1-rev"', board_inner)
    board_inner_2 = re.sub(r"url\(#(board-[a-z-]+)\)", r"url(#\1-rev)", board_inner_2)
    if "-rev" not in board_inner_2:
        raise SystemExit("[FAIL] 未能为第二份棋盘实例重命名 gradient/filter id")

    html = read_utf8(TEMPLATE)
    replacements = [
        ("<!--@PIECE_SPRITES@-->", sprites),
        ("<!--@BOARD_SVG@-->", board_inner),
        ("<!--@BOARD_SVG_REVIEW@-->", board_inner_2),
        ("<!--@PIECES@-->", pieces_layer),
        ("<!--@PIECES_REVIEW@-->", pieces_layer),
    ]
    for token, value in replacements:
        if token not in html:
            raise SystemExit(f"[FAIL] 模板缺少占位符：{token}")
        html = html.replace(token, value)

    left = re.findall(r"<!--@[A-Z_]+@-->", html)
    if left:
        raise SystemExit(f"[FAIL] 仍有未替换的占位符：{left}")

    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(html, encoding="utf-8")

    size = OUTPUT.stat().st_size
    print(f" 写出 {OUTPUT.relative_to(ROOT)}  ({size:,} 字节)")
    print(f"  棋子 sprite {len(PIECE_FILES)} 个 / 棋盘实例 2 份 / 演示局面棋子 {len(POSITION)} 枚")
    print("build_prototype: 完成")
    return 0


if __name__ == "__main__":
    sys.exit(main())
