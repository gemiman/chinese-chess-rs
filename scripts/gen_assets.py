#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""弈道 · chinese-chess-rs —— 视觉素材与设计令牌生成器。

生成产物（全部位于 <repo>/assets/ 下）：
    board/board-classic.svg     标准 9x10 棋盘
    board/board-coords.svg      带 ICCS 坐标的调试棋盘
    pieces/*.svg                14 种棋子（红 7 + 黑 7）
    tokens/design-tokens.json   设计令牌
    tokens/colors.md            配色规范 + WCAG 对比度实算表

脚本幂等：不使用随机数、不依赖时间，重复运行输出逐字节一致。
运行： python scripts/gen_assets.py
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
import xml.etree.ElementTree as ET

# --------------------------------------------------------------------------
# 路径
# --------------------------------------------------------------------------

ROOT_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS_DIR = os.path.join(ROOT_DIR, "assets")
BOARD_DIR = os.path.join(ASSETS_DIR, "board")
PIECE_DIR = os.path.join(ASSETS_DIR, "pieces")
TOKEN_DIR = os.path.join(ASSETS_DIR, "tokens")

# --------------------------------------------------------------------------
# 调色板（唯一定义源，棋盘 / 棋子 / 令牌 / 文档全部由此派生）
# --------------------------------------------------------------------------

# ---- 棋盘：打磨过的浅石绿石板 ----
BOARD_SIDE = "#6B7661"       # 板侧厚度 / 盘面暗部
BOARD_FACE_HI = "#DCE2D0"    # 盘面高光（左上受光）
BOARD_FACE_LO = "#AFB79F"    # 盘面暗角（右下背光）
BOARD_FRAME_HI = "#AEB8A0"   # 凸起厚石框的受光面
BOARD_FRAME_LO = "#7C886E"   # 厚石框的背光面
LINE = "#4E5642"             # 盘面刻线
LINE_STRONG = "#6F7A61"      # 内外边框线

# ---- 棋子：暖金木牌。红黑材质相同，只靠字色区分 ----
PIECE_SIDE = "#9F7638"       # 侧壁（厚度）
PIECE_TOP_HI = "#F6E2B4"     # 顶面高光（左上）
PIECE_TOP_LO = "#DCB673"     # 顶面中调
PIECE_TOP_EDGE = "#BC9250"   # 顶面边缘倒角
PIECE_RING = "#8E6829"       # 外圈线
PIECE_GROOVE = "#C39C58"     # 内凹槽线
PIECE_WELL = "#EDD5A0"       # 内凹底

RED_STROKE = "#B3282D"       # 红方字色
BLACK_STROKE = "#1F2430"     # 黑方字色

STATE = {
    "selected": "#F5A623",
    "selected-alpha": "0.55",
    "legal-move": "#2ECC71",
    "capture-target": "#E74C3C",
    "last-move": "#3498DB",
    "check": "#D0021B",
    "hint-arrow": "#8E44AD",
}

COACH = {
    "best": "#27AE60",      # 优秀
    "good": "#2E86DE",      # 良好
    "dubious": "#F39C12",   # 可疑
    "blunder": "#E74C3C",   # 失误
    "missed": "#8E44AD",    # 漏着
}

UI = {
    "background": "#F7F3EC",
    "panel": "#FFFFFF",
    "card": "#FFFDF8",
    "border": "#D8CBB4",
    "text-primary": "#1F2430",
    "text-secondary": "#5A6472",
}

FONT_ZH = '"KaiTi","STKaiti","Noto Serif SC",serif'
FONT_MONO = '"Consolas","SF Mono","DejaVu Sans Mono",monospace'

# --------------------------------------------------------------------------
# 通用工具
# --------------------------------------------------------------------------


def n(v) -> str:
    """紧凑数字格式化：整数省略小数点，浮点去掉尾随零。确定性输出。"""
    if isinstance(v, bool):
        raise TypeError("bool 不是合法坐标")
    f = float(v)
    if f.is_integer():
        return str(int(f))
    return f"{f:.4f}".rstrip("0").rstrip(".")


def esc(text: str) -> str:
    """文本节点转义。"""
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def esc_attr(text: str) -> str:
    """属性值转义（含双引号，保证字体系列里的 " 可安全内嵌）。"""
    return esc(text).replace('"', "&quot;")


def hex_to_rgb(value: str):
    v = value.strip().lstrip("#")
    return tuple(int(v[i:i + 2], 16) for i in (0, 2, 4))


def rgb_to_hex(rgb) -> str:
    return "#%02X%02X%02X" % tuple(int(round(c)) for c in rgb)


def composite_over(fg_hex: str, alpha: float, bg_hex: str) -> str:
    """alpha 合成：fg 以 alpha 叠加在 bg 之上，返回不透明结果色。"""
    fg, bg = hex_to_rgb(fg_hex), hex_to_rgb(bg_hex)
    return rgb_to_hex([fg[i] * alpha + bg[i] * (1.0 - alpha) for i in range(3)])


def _srgb_channel(c: float) -> float:
    c = c / 255.0
    return c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4


def relative_luminance(hex_color: str) -> float:
    r, g, b = (_srgb_channel(c) for c in hex_to_rgb(hex_color))
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast_ratio(fg_hex: str, bg_hex: str) -> float:
    l1, l2 = relative_luminance(fg_hex), relative_luminance(bg_hex)
    if l1 < l2:
        l1, l2 = l2, l1
    return (l1 + 0.05) / (l2 + 0.05)


def wcag_level(ratio: float) -> str:
    if ratio >= 4.5:
        return "AA"
    if ratio >= 3.0:
        return "AA Large"
    return "不达标"


WRITTEN: list[tuple[str, int, str]] = []  # (相对路径, 字节数, sha256)


def write_text(rel_path: str, content: str) -> None:
    """UTF-8 + LF 写盘，并登记字节数与哈希。"""
    abspath = os.path.join(ROOT_DIR, rel_path)
    os.makedirs(os.path.dirname(abspath), exist_ok=True)
    data = content.encode("utf-8")
    with open(abspath, "wb") as fh:
        fh.write(data)
    WRITTEN.append((rel_path.replace("\\", "/"), len(data), hashlib.sha256(data).hexdigest()))


def svg_doc(width: int, height: int, body: list[str]) -> str:
    head = (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<svg xmlns="http://www.w3.org/2000/svg" '
        f'width="{width}" height="{height}" viewBox="0 0 {width} {height}">\n'
    )
    return head + "\n".join(body) + "\n</svg>\n"


# --------------------------------------------------------------------------
# 棋盘几何（严格参数，勿改）
# --------------------------------------------------------------------------

BOARD_W, BOARD_H = 560, 620
MARGIN, CELL = 40, 60
N_COLS, N_ROWS = 9, 10          # col 0..8, row 0..9
X0, X1 = MARGIN, MARGIN + (N_COLS - 1) * CELL          # 40 / 520
Y0, Y1 = MARGIN, MARGIN + (N_ROWS - 1) * CELL          # 40 / 580
RIVER_TOP, RIVER_BOTTOM = MARGIN + 4 * CELL, MARGIN + 5 * CELL   # 280 / 340

LINE_W = 1.6
BORDER_W = 2.4
MARK_GAP, MARK_LEN, MARK_W = 6, 8, 1.4
RIVER_FONT_SIZE = 26
RIVER_Y = 310
RIVER_LEFT_X, RIVER_RIGHT_X = 180, 380


def px(col: int) -> int:
    return MARGIN + col * CELL


def py(row: int) -> int:
    return MARGIN + row * CELL


# --- 横线：10 条 ----------------------------------------------------------
H_LINES = [(X0, py(r), X1, py(r)) for r in range(N_ROWS)]

# --- 竖线：9 条，中间 7 条被河界断为 2 段 => 16 段 -------------------------
V_LINES: list[tuple[int, int, int, int]] = []
for c in range(N_COLS):
    x = px(c)
    if c in (0, N_COLS - 1):
        V_LINES.append((x, Y0, x, Y1))
    else:
        V_LINES.append((x, Y0, x, RIVER_TOP))
        V_LINES.append((x, RIVER_BOTTOM, x, Y1))

# --- 九宫斜线：4 条 -------------------------------------------------------
PALACE_DIAGONALS = [
    (px(3), py(0), px(5), py(2)),
    (px(5), py(0), px(3), py(2)),
    (px(3), py(7), px(5), py(9)),
    (px(5), py(7), px(3), py(9)),
]

# --- 兵位 10 点 / 炮位 4 点 -----------------------------------------------
SOLDIER_POINTS = [
    (0, 3), (2, 3), (4, 3), (6, 3), (8, 3),
    (0, 6), (2, 6), (4, 6), (6, 6), (8, 6),
]
CANNON_POINTS = [(1, 2), (7, 2), (1, 7), (7, 7)]


def mark_corners(col: int, row: int) -> list[list[tuple[int, int]]]:
    """落子点周围的十字折角；边线点只画朝向棋盘内侧的一侧。"""
    cx, cy = px(col), py(row)
    corners: list[list[tuple[int, int]]] = []
    if col != 0:                                   # 左侧折角
        lx = cx - MARK_GAP
        # 先画水平段（由外向内），再折出竖直段
        corners.append([(lx - MARK_LEN, cy - MARK_GAP), (lx, cy - MARK_GAP), (lx, cy - MARK_GAP - MARK_LEN)])
        corners.append([(lx - MARK_LEN, cy + MARK_GAP), (lx, cy + MARK_GAP), (lx, cy + MARK_GAP + MARK_LEN)])
    if col != N_COLS - 1:                          # 右侧折角
        rx = cx + MARK_GAP
        corners.append([(rx + MARK_LEN, cy - MARK_GAP), (rx, cy - MARK_GAP), (rx, cy - MARK_GAP - MARK_LEN)])
        corners.append([(rx + MARK_LEN, cy + MARK_GAP), (rx, cy + MARK_GAP), (rx, cy + MARK_GAP + MARK_LEN)])
    return corners


MARK_GROUPS = [
    ("soldier", col, row, mark_corners(col, row)) for col, row in SOLDIER_POINTS
] + [
    ("cannon", col, row, mark_corners(col, row)) for col, row in CANNON_POINTS
]
TOTAL_CORNERS = sum(len(g[3]) for g in MARK_GROUPS)

BORDER_INNER = (34, 34, 492, 552)
BORDER_OUTER = (28, 28, 504, 564)


# --------------------------------------------------------------------------
# 棋盘 SVG
# --------------------------------------------------------------------------


# 石板质感参数。
# 频率取低值 → 大块缓慢的斑驳（打磨过的石头），频率高会变成细密砂粒，1:1 看很脏。
BOARD_STONE_FREQ = 0.018
BOARD_STONE_RELIEF = 1.5
BOARD_SPEC_CONST = 0.26
BOARD_SPEC_EXP = 34


def _defs_block() -> list[str]:
    """棋盘的渐变与滤镜。

    石板质感：feDiffuseLighting 从 turbulence 噪声推出法线做浮雕，
    再叠一层 feSpecularLighting 得到打磨后的高光。
    刻线：双 feDropShadow 做出「上暗下亮」的凹槽感。
    """
    return [
        "  <defs>",
        '    <radialGradient id="board-face" cx="34%" cy="26%" r="92%">',
        f'      <stop offset="0%" stop-color="{BOARD_FACE_HI}"/>',
        f'      <stop offset="100%" stop-color="{BOARD_FACE_LO}"/>',
        "    </radialGradient>",
        '    <linearGradient id="board-frame" x1="0" y1="0" x2="1" y2="1">',
        f'      <stop offset="0%" stop-color="{BOARD_FRAME_HI}"/>',
        f'      <stop offset="100%" stop-color="{BOARD_FRAME_LO}"/>',
        "    </linearGradient>",
        '    <filter id="board-stone" x="0" y="0" width="100%" height="100%">',
        f'      <feTurbulence type="fractalNoise" baseFrequency="{BOARD_STONE_FREQ}" '
        'numOctaves="4" seed="11" stitchTiles="stitch" result="n"/>',
        f'      <feDiffuseLighting in="n" lighting-color="#FFFFFF" '
        f'surfaceScale="{BOARD_STONE_RELIEF}" diffuseConstant="1.15" result="lit">',
        '        <feDistantLight azimuth="235" elevation="52"/>',
        "      </feDiffuseLighting>",
        '      <feComposite in="lit" in2="SourceGraphic" operator="arithmetic" '
        'k1="1.08" k2="0.01" k3="0.01" k4="0" result="base"/>',
        f'      <feSpecularLighting in="n" surfaceScale="{BOARD_STONE_RELIEF}" '
        f'specularConstant="{BOARD_SPEC_CONST}" specularExponent="{BOARD_SPEC_EXP}" '
        'lighting-color="#FFFFFF" result="spec">',
        '        <feDistantLight azimuth="235" elevation="52"/>',
        "      </feSpecularLighting>",
        '      <feComposite in="spec" in2="base" operator="arithmetic" '
        'k1="0" k2="1" k3="1" k4="0"/>',
        "    </filter>",
        '    <filter id="board-carve" x="-4%" y="-4%" width="108%" height="108%">',
        '      <feDropShadow dx="0" dy="-0.9" stdDeviation="0.35" '
        'flood-color="#2A3020" flood-opacity="0.55"/>',
        '      <feDropShadow dx="0" dy="1" stdDeviation="0.35" '
        'flood-color="#F2F6E6" flood-opacity="0.42"/>',
        "    </filter>",
        "  </defs>",
    ]


def _line(x1, y1, x2, y2, color, width, cls=None) -> str:
    attr = f' class="{cls}"' if cls else ""
    return (
        f'  <line{attr} x1="{n(x1)}" y1="{n(y1)}" x2="{n(x2)}" y2="{n(y2)}" '
        f'stroke="{color}" stroke-width="{n(width)}" stroke-linecap="square"/>'
    )


def _board_geometry_elements() -> list[str]:
    """棋盘主体（不含底板）。"""
    out: list[str] = []

    # 内嵌盘面：石板质感由 board-stone 滤镜推出浮雕与高光
    bx, by, bw, bh = BORDER_OUTER
    out.append(
        f'  <rect x="{n(bx)}" y="{n(by)}" width="{n(bw)}" height="{n(bh)}" '
        f'rx="4" fill="url(#board-face)" filter="url(#board-stone)"/>'
    )
    # 盘面四周内阴影，让可下区域看起来嵌在石框里
    out.append(
        f'  <rect x="{n(bx)}" y="{n(by)}" width="{n(bw)}" height="{n(bh)}" '
        f'rx="4" fill="none" stroke="{BOARD_SIDE}" stroke-width="7" opacity="0.38"/>'
    )

    # 内外边框
    for (x, y, w, h) in (BORDER_INNER, BORDER_OUTER):
        out.append(
            f'  <rect x="{n(x)}" y="{n(y)}" width="{n(w)}" height="{n(h)}" '
            f'fill="none" stroke="{LINE_STRONG}" stroke-width="{n(BORDER_W)}" rx="3"/>'
        )

    # 刻线统一挂 board-carve 滤镜，做出「上暗下亮」的凹槽
    carve = ' filter="url(#board-carve)"'

    # 横线
    out.append(f'  <g class="grid-horizontal"{carve}>')
    for (x1, y1, x2, y2) in H_LINES:
        out.append("  " + _line(x1, y1, x2, y2, LINE, LINE_W, "grid-line"))
    out.append("  </g>")

    # 竖线
    out.append(f'  <g class="grid-vertical"{carve}>')
    for (x1, y1, x2, y2) in V_LINES:
        out.append("  " + _line(x1, y1, x2, y2, LINE, LINE_W, "grid-line"))
    out.append("  </g>")

    # 九宫斜线
    out.append(f'  <g class="palace"{carve}>')
    for (x1, y1, x2, y2) in PALACE_DIAGONALS:
        out.append("  " + _line(x1, y1, x2, y2, LINE, LINE_W, "palace-line"))
    out.append("  </g>")

    # 兵炮位标记
    out.append(f'  <g class="marks"{carve}>')
    for kind, col, row, corners in MARK_GROUPS:
        out.append(
            f'    <g class="mark-point" data-kind="{kind}" data-col="{col}" data-row="{row}">'
        )
        for pts in corners:
            coords = " ".join(f"{n(x)},{n(y)}" for x, y in pts)
            out.append(
                f'      <polyline class="mark-corner" points="{coords}" fill="none" '
                f'stroke="{LINE}" stroke-width="{n(MARK_W)}" stroke-linecap="square"/>'
            )
        out.append("    </g>")
    out.append("  </g>")

    # 河界文字也走刻线滤镜，做出嵌进石头里的效果
    out.append(f'  <g class="river-text"{carve}>')
    for x, text in ((RIVER_LEFT_X, "楚 河"), (RIVER_RIGHT_X, "汉 界")):
        out.append(
            f'  <text x="{x}" y="{RIVER_Y}" text-anchor="middle" dominant-baseline="central" '
            f'font-family="{esc_attr(FONT_ZH)}" font-size="{RIVER_FONT_SIZE}" '
            f'letter-spacing="8" fill="{LINE}">{esc(text)}</text>'
        )
    out.append("  </g>")

    return out


def build_board_classic() -> str:
    body: list[str] = []
    body += _defs_block()
    # 板身厚度：整块石板下移一段，露出底部侧边
    body.append(
        f'  <rect x="0" y="7" width="{BOARD_W}" height="{BOARD_H}" rx="12" '
        f'fill="{BOARD_SIDE}"/>'
    )
    # 凸起的厚石框，内嵌盘面画在其上
    body.append(
        f'  <rect x="0" y="0" width="{BOARD_W}" height="{BOARD_H}" rx="12" '
        f'fill="url(#board-frame)"/>'
    )
    body += _board_geometry_elements()
    return svg_doc(BOARD_W, BOARD_H, body)


def build_board_coords() -> str:
    w, h = 590, 630
    body: list[str] = []
    body += _defs_block()
    body.append(
        f'  <rect x="0" y="7" width="{w}" height="{h}" rx="12" fill="{BOARD_SIDE}"/>'
    )
    body.append(
        f'  <rect x="0" y="0" width="{w}" height="{h}" rx="12" fill="url(#board-frame)"/>'
    )
    body += _board_geometry_elements()

    body.append('  <g class="coords">')
    for c in range(N_COLS):
        body.append(
            f'  <text class="coord-col" x="{px(c)}" y="600" text-anchor="middle" '
            f'dominant-baseline="central" font-family="{esc_attr(FONT_MONO)}" '
            f'font-size="11" fill="#A0522D">{chr(ord("a") + c)}</text>'
        )
    for r in range(N_ROWS):
        body.append(
            f'  <text class="coord-row" x="18" y="{py(r)}" text-anchor="middle" '
            f'dominant-baseline="central" font-family="{esc_attr(FONT_MONO)}" '
            f'font-size="11" fill="#A0522D">{r}</text>'
        )
    body.append("  </g>")

    return svg_doc(w, h, body)


# --------------------------------------------------------------------------
# 棋子 SVG
# --------------------------------------------------------------------------

PIECE_R = 29
PIECE_INNER_RING_R = 23.5
PIECE_INNER_FILL_R = 21
PIECE_STROKE_W = 2.4
PIECE_RING_W = 1.1
PIECE_FONT_SIZE = 30

PIECE_SHADOW_DY = 3.6
PIECE_SHADOW_BLUR = 3.2
PIECE_SHADOW_OPACITY = 0.42

FACTIONS = {
    "red": {"text": RED_STROKE},
    "black": {"text": BLACK_STROKE},
}

PIECES = [
    ("red_king", "帥", "red"),
    ("red_advisor", "仕", "red"),
    ("red_elephant", "相", "red"),
    ("red_horse", "傌", "red"),
    ("red_chariot", "俥", "red"),
    ("red_cannon", "炮", "red"),
    ("red_pawn", "兵", "red"),
    ("black_general", "將", "black"),
    ("black_advisor", "士", "black"),
    ("black_elephant", "象", "black"),
    ("black_horse", "馬", "black"),
    ("black_chariot", "車", "black"),
    ("black_cannon", "砲", "black"),
    ("black_pawn", "卒", "black"),
]


def build_piece(name: str, char: str, faction: str) -> str:
    """暖金木牌：侧壁撑出厚度，顶面走径向渐变做出弧面高光，字是凹刻的。

    渐变与滤镜的 id 带棋子名，保证 14 枚棋子内联到同一文档时不冲突。
    """
    p = FACTIONS[faction]
    dome = f"piece-dome-{name}"
    shadow = f"piece-shadow-{name}"
    body = [
        "  <defs>",
        f'    <radialGradient id="{dome}" cx="34%" cy="26%" r="78%">',
        f'      <stop offset="0%" stop-color="{PIECE_TOP_HI}"/>',
        f'      <stop offset="78%" stop-color="{PIECE_TOP_LO}"/>',
        f'      <stop offset="100%" stop-color="{PIECE_TOP_EDGE}"/>',
        "    </radialGradient>",
        f'    <filter id="{shadow}" x="-30%" y="-30%" width="160%" height="175%">',
        f'      <feDropShadow dx="0" dy="{n(PIECE_SHADOW_DY)}" '
        f'stdDeviation="{n(PIECE_SHADOW_BLUR)}" flood-color="#000000" '
        f'flood-opacity="{n(PIECE_SHADOW_OPACITY)}"/>',
        "    </filter>",
        "  </defs>",
        # 1) 侧壁：圆牌整体下移一段，露出底部一道深色侧边，形成厚度
        f'  <circle cx="32" cy="{n(32 + PIECE_SHADOW_DY)}" r="{n(PIECE_R)}" '
        f'fill="{PIECE_SIDE}" filter="url(#{shadow})"/>',
        # 2) 顶面 + 外圈线
        f'  <circle cx="32" cy="32" r="{n(PIECE_R)}" fill="url(#{dome})" '
        f'stroke="{PIECE_RING}" stroke-width="{n(PIECE_STROKE_W)}"/>',
        # 3) 内凹槽线
        f'  <circle cx="32" cy="32" r="{n(PIECE_INNER_RING_R)}" fill="none" '
        f'stroke="{PIECE_GROOVE}" stroke-width="{n(PIECE_RING_W)}"/>',
        # 4) 内凹底
        f'  <circle cx="32" cy="32" r="{n(PIECE_INNER_FILL_R)}" fill="{PIECE_WELL}"/>',
        # 凹刻字：浅色描边垫在字底下，形成凿刻的边
        f'  <text x="32" y="33" text-anchor="middle" dominant-baseline="central" '
        f'font-family="{esc_attr(FONT_ZH)}" font-size="{PIECE_FONT_SIZE}" font-weight="700" '
        f'fill="{p["text"]}" stroke="{PIECE_TOP_HI}" stroke-width="1.15" paint-order="stroke">'
        f'{esc(char)}</text>',
    ]
    return svg_doc(64, 64, body)


# --------------------------------------------------------------------------
# 设计令牌
# --------------------------------------------------------------------------


def build_tokens() -> str:
    tokens = {
        "board": {
            "side": BOARD_SIDE,
            "face-highlight": BOARD_FACE_HI,
            "face-shadow": BOARD_FACE_LO,
            "frame-highlight": BOARD_FRAME_HI,
            "frame-shadow": BOARD_FRAME_LO,
            "line": LINE,
            "line-strong": LINE_STRONG,
            "river-text": LINE,
            "grid-width": "1.6px",
            "border-width": "2.4px",
            "mark-width": "1.4px",
            "cell": "60px",
            "margin": "40px",
            "cols": 9,
            "rows": 10,
            "view-width": "560px",
            "view-height": "620px",
        },
        "piece": {
            "material": {
                "side": PIECE_SIDE,
                "top-highlight": PIECE_TOP_HI,
                "top-shadow": PIECE_TOP_LO,
                "top-edge": PIECE_TOP_EDGE,
                "ring": PIECE_RING,
                "groove": PIECE_GROOVE,
                "well": PIECE_WELL,
            },
            "red": {"text": RED_STROKE},
            "black": {"text": BLACK_STROKE},
            "size": "64px",
            "radius": "29px",
            "ring-radius": "23.5px",
            "inner-radius": "21px",
            "stroke-width": "2.4px",
            "ring-width": "1.1px",
            "font-size": "30px",
            "shadow-offset-y": "3.6px",
            "shadow-blur": "3.2px",
        },
        "state": {
            "selected": STATE["selected"],
            "selected-alpha": STATE["selected-alpha"],
            "selected-overlay": "rgba(245,166,35,0.55)",
            "legal-move": STATE["legal-move"],
            "capture-target": STATE["capture-target"],
            "last-move": STATE["last-move"],
            "check": STATE["check"],
            "hint-arrow": STATE["hint-arrow"],
        },
        "coach": {
            "best": COACH["best"],
            "good": COACH["good"],
            "dubious": COACH["dubious"],
            "blunder": COACH["blunder"],
            "missed": COACH["missed"],
        },
        "ui": {
            "background": UI["background"],
            "panel": UI["panel"],
            "card": UI["card"],
            "border": UI["border"],
            "text-primary": UI["text-primary"],
            "text-secondary": UI["text-secondary"],
            "radius-sm": "4px",
            "radius-md": "8px",
            "radius-lg": "16px",
            "shadow-sm": "0 1px 2px rgba(31,36,48,0.08)",
            "shadow-md": "0 4px 12px rgba(31,36,48,0.12)",
            "shadow-lg": "0 10px 28px rgba(31,36,48,0.16)",
            "duration-fast": "120ms",
            "duration-base": "220ms",
            "duration-slow": "360ms",
            "easing-standard": "cubic-bezier(0.2,0,0.2,1)",
            "easing-decelerate": "cubic-bezier(0,0,0.2,1)",
            "easing-emphasized": "cubic-bezier(0.2,0,0,1)",
        },
        "typography": {
            "font-family-zh": FONT_ZH,
            "font-family-mono": FONT_MONO,
            "font-size-1": "12px",
            "font-size-2": "14px",
            "font-size-3": "16px",
            "font-size-4": "20px",
            "font-size-5": "26px",
            "font-size-6": "34px",
            "font-weight-regular": "400",
            "font-weight-bold": "700",
        },
    }
    return json.dumps(tokens, indent=2, ensure_ascii=False) + "\n"


# --------------------------------------------------------------------------
# colors.md —— 对比度全部由 Python 实算
# --------------------------------------------------------------------------


def advice(level: str, kind: str) -> str:
    """按「文本 / 图形」与达标等级给出处置建议。"""
    if level == "AA":
        return "可用于正文文本。"
    if kind == "装饰":
        return "装饰性元素，不受 WCAG 1.4.11 约束；但不可作为唯一的结构区分手段。"
    if kind == "图形":
        if level == "AA Large":
            return "非文本图形，满足 WCAG 1.4.11 的 3:1 要求。"
        return "非文本图形也未达 3:1，须叠加形状/描边/图例/文字标签，不可仅靠颜色区分语义。"
    if level == "AA Large":
        return "仅可作 ≥18pt 或 ≥14pt 粗体大字，不可承载正文。"
    return "不可作文字色，需要文字时改用 `ui.text-primary`。"


def build_colors_md() -> str:
    # (分组, 前景, 背景, 说明, 用途分类)
    pairs = [
        # 棋盘
        ("棋盘", LINE, BOARD_FACE_LO, "棋格刻线 / 河界文字", "大字"),
        ("棋盘", LINE_STRONG, BOARD_FACE_LO, "内外边框线", "图形"),
        ("棋盘", "#A0522D", BOARD_FACE_LO, "坐标标注文字", "小字"),
        # 棋子
        ("棋子", RED_STROKE, PIECE_TOP_HI, "红方棋字 on 牌面高光处", "大字"),
        ("棋子", RED_STROKE, PIECE_WELL, "红方棋字 on 内凹底", "大字"),
        ("棋子", BLACK_STROKE, PIECE_TOP_HI, "黑方棋字 on 牌面高光处", "大字"),
        ("棋子", BLACK_STROKE, PIECE_WELL, "黑方棋字 on 内凹底", "大字"),
        # 状态色（叠加在棋盘石面上）
        ("状态", STATE["selected"], BOARD_FACE_LO, "选中高亮（纯色）", "图形"),
        (
            "状态",
            composite_over(STATE["selected"], 0.55, BOARD_FACE_LO),
            BOARD_FACE_LO,
            "选中高亮（55% 合成后）",
            "图形",
        ),
        ("状态", STATE["legal-move"], BOARD_FACE_LO, "合法落点", "图形"),
        ("状态", STATE["capture-target"], BOARD_FACE_LO, "吃子目标", "图形"),
        ("状态", STATE["last-move"], BOARD_FACE_LO, "上一步落点", "图形"),
        ("状态", STATE["check"], BOARD_FACE_LO, "将军警告", "图形"),
        ("状态", STATE["hint-arrow"], BOARD_FACE_LO, "推荐着法箭头", "图形"),
        # 战法评价色阶（浅底）
        ("战法评价", COACH["best"], UI["panel"], "优秀", "小字"),
        ("战法评价", COACH["good"], UI["panel"], "良好", "小字"),
        ("战法评价", COACH["dubious"], UI["panel"], "可疑", "小字"),
        ("战法评价", COACH["blunder"], UI["panel"], "失误", "小字"),
        ("战法评价", COACH["missed"], UI["panel"], "漏着", "小字"),
        # 战法评价徽标（实底反白）
        ("战法评价徽标", "#FFFFFF", COACH["best"], "白字 on 优秀绿", "小字"),
        ("战法评价徽标", "#FFFFFF", COACH["good"], "白字 on 良好蓝", "小字"),
        ("战法评价徽标", "#FFFFFF", COACH["dubious"], "白字 on 可疑橙", "小字"),
        ("战法评价徽标", "#FFFFFF", COACH["blunder"], "白字 on 失误红", "小字"),
        ("战法评价徽标", "#FFFFFF", COACH["missed"], "白字 on 漏着紫", "小字"),
        # UI
        ("界面", UI["text-primary"], UI["background"], "主文本 on 背景", "小字"),
        ("界面", UI["text-primary"], UI["panel"], "主文本 on 面板", "小字"),
        ("界面", UI["text-primary"], UI["card"], "主文本 on 卡片", "小字"),
        ("界面", UI["text-secondary"], UI["background"], "次文本 on 背景", "小字"),
        ("界面", UI["text-secondary"], UI["panel"], "次文本 on 面板", "小字"),
        ("界面", UI["border"], UI["panel"], "边框 on 面板", "装饰"),
        ("界面", "#FFFFFF", UI["text-primary"], "白字 on 深色按钮", "小字"),
    ]

    rows = []
    counts = {"AA": 0, "AA Large": 0, "不达标": 0}
    for group, fg, bg, note, kind in pairs:
        ratio = contrast_ratio(fg, bg)
        level = wcag_level(ratio)
        counts[level] += 1
        rows.append((group, fg, bg, note, kind, ratio, level))

    L = []
    A = L.append
    A("# 配色规范与 WCAG 对比度校验")
    A("")
    A("> 本文件由 `scripts/gen_assets.py` 自动生成，请勿手改。")
    A("> 所有对比度数值均由脚本按 WCAG 2.1 相对亮度公式**实算**得出，非估算。")
    A("")
    A("## 1. 计算公式")
    A("")
    A("```")
    A("c' = c/255")
    A("c_lin = c'/12.92                    若 c' <= 0.03928")
    A("c_lin = ((c'+0.055)/1.055) ** 2.4   否则")
    A("L = 0.2126*R + 0.7152*G + 0.0722*B")
    A("contrast = (L_bright + 0.05) / (L_dark + 0.05)")
    A("```")
    A("")
    A("达标口径：**AA** ≥ 4.5（正文文本）；**AA Large** ≥ 3.0（≥18pt 或 ≥14pt 粗体，"
      "以及图表等非文本图形）；**不达标** < 3.0。")
    A("")

    # 分组色板
    def swatch_table(title: str, entries):
        A(f"## {title}")
        A("")
        A("| 令牌 | 色值 | 用途 |")
        A("|---|---|---|")
        for name, val, use in entries:
            A(f"| `{name}` | `{val}` | {use} |")
        A("")

    swatch_table("2. 棋盘配色", [
        ("board.side", BOARD_SIDE, "板侧厚度 / 盘面暗部"),
        ("board.face-highlight", BOARD_FACE_HI, "盘面高光（径向渐变起点）"),
        ("board.face-shadow", BOARD_FACE_LO, "盘面暗角（径向渐变终点）"),
        ("board.frame-highlight", BOARD_FRAME_HI, "凸起厚石框受光面"),
        ("board.frame-shadow", BOARD_FRAME_LO, "凸起厚石框背光面"),
        ("board.line", LINE, "棋格刻线 / 兵炮位标记 / 河界文字"),
        ("board.line-strong", LINE_STRONG, "内外边框线"),
    ])
    swatch_table("3. 棋子配色", [
        ("piece.material.side", PIECE_SIDE, "侧壁（厚度所在）"),
        ("piece.material.top-highlight", PIECE_TOP_HI, "牌面高光（径向渐变起点）"),
        ("piece.material.top-shadow", PIECE_TOP_LO, "牌面中调（渐变 78% 处）"),
        ("piece.material.top-edge", PIECE_TOP_EDGE, "牌面边缘倒角（渐变终点）"),
        ("piece.material.ring", PIECE_RING, "外圈线"),
        ("piece.material.groove", PIECE_GROOVE, "内凹槽线"),
        ("piece.material.well", PIECE_WELL, "内凹底"),
        ("piece.red.text", RED_STROKE, "红方字色"),
        ("piece.black.text", BLACK_STROKE, "黑方字色"),
    ])
    swatch_table("4. 状态色", [
        ("state.selected", STATE["selected"], "选中高亮（叠加透明度 0.55）"),
        ("state.legal-move", STATE["legal-move"], "合法落点"),
        ("state.capture-target", STATE["capture-target"], "吃子目标"),
        ("state.last-move", STATE["last-move"], "上一步落点"),
        ("state.check", STATE["check"], "将军警告"),
        ("state.hint-arrow", STATE["hint-arrow"], "推荐着法箭头"),
    ])
    swatch_table("5. 战法评价色阶（好 → 坏）", [
        ("coach.best", COACH["best"], "优秀"),
        ("coach.good", COACH["good"], "良好"),
        ("coach.dubious", COACH["dubious"], "可疑"),
        ("coach.blunder", COACH["blunder"], "失误"),
        ("coach.missed", COACH["missed"], "漏着"),
    ])
    swatch_table("6. 界面基础色", [
        ("ui.background", UI["background"], "页面背景"),
        ("ui.panel", UI["panel"], "面板底色"),
        ("ui.card", UI["card"], "卡片底色"),
        ("ui.border", UI["border"], "分隔线 / 描边"),
        ("ui.text-primary", UI["text-primary"], "主文本"),
        ("ui.text-secondary", UI["text-secondary"], "次文本"),
    ])

    A("## 7. WCAG 对比度实测总表")
    A("")
    A("| # | 分组 | 前景 | 背景 | 组合说明 | 对比度 | 等级 | 判定 |")
    A("|---|---|---|---|---|---|---|---|")
    for i, (group, fg, bg, note, kind, ratio, level) in enumerate(rows, 1):
        verdict = {"AA": "✅ 达标（AA）", "AA Large": "⚠️ 仅 AA Large", "不达标": "❌ 不达标"}[level]
        A(f"| {i} | {group} | `{fg}` | `{bg}` | {note} | **{ratio:.2f}:1** | {level} | {verdict} |")
    A("")

    A("## 8. 结论摘要")
    A("")
    A(f"- 共评估 **{len(rows)}** 组前景/背景组合。")
    A(f"- 达到 **AA**（≥ 4.5:1）：**{counts['AA']}** 组。")
    A(f"- 仅达 **AA Large**（3.0 ~ 4.5:1）：**{counts['AA Large']}** 组。")
    A(f"- **不达标**（< 3.0:1）：**{counts['不达标']}** 组。")
    A("")

    weak = [r for r in rows if r[6] != "AA"]
    if weak:
        A("未达 AA（4.5:1）的组合及处置建议：")
        A("")
        A("| 分组 | 组合说明 | 前景 on 背景 | 对比度 | 等级 | 处置建议 |")
        A("|---|---|---|---|---|---|")
        for group, fg, bg, note, kind, ratio, level in weak:
            A(f"| {group} | {note} | `{fg}` on `{bg}` | {ratio:.2f}:1 | {level} | {advice(level, kind)} |")
        A("")
        A("## 9. 兜底策略")
        A("")
        A("- **状态色一律不作文字色。** 选中/落点/吃子/上一步/将军等提示以半透明色块、"
          "描边圆环或角标呈现，文字始终使用 `ui.text-primary`，从根本上规避对比度风险。")
        A("- **非文本图形口径。** 按 WCAG 1.4.11，承载语义的图形需 ≥ 3:1。第 8 节中标注"
          "「也未达 3:1」的状态色必须叠加形状差异（如实心/空心、圆点/叉号）或文字标签，"
          "不能仅靠颜色区分——这同时也是对色觉障碍用户的要求。")
        A("- **战法评价色阶。** 文字场景统一使用实底反白徽标，并选取满足 AA 的底色组合"
          "（目前 `coach.missed #8E44AD` 达标）；`coach.dubious #F39C12` 与 "
          "`coach.best #27AE60` 需改用深色文本或加深底色后再承载文字。")
        A("- **棋盘线/河界文字（4.25:1）与坐标标注（4.08:1）** 属大字或图形口径，"
          "实际应用字号为 26px / 11px；其中 11px 坐标标注仅作为开发期调试用途，"
          "正式产品界面不对外暴露。")
        A("- **`ui.border`（1.60:1）** 为装饰性分隔线，不受 WCAG 1.4.11 约束；"
          "但页面结构不能仅靠它区分，须辅以间距或标题层级。")
        A("")

    return "\n".join(L)


# --------------------------------------------------------------------------
# 校验：解析全部 SVG 并做几何断言
# --------------------------------------------------------------------------


def _tag(elem) -> str:
    return elem.tag.split("}")[-1]


def count_elements(path: str) -> dict[str, int]:
    root = ET.parse(path).getroot()
    counter: dict[str, int] = {}
    for elem in root.iter():
        counter[_tag(elem)] = counter.get(_tag(elem), 0) + 1
    return counter


def verify_svg(path: str) -> dict:
    """解析 + 几何断言。返回统计信息。"""
    ET.parse(path)  # 合法 XML，否则抛异常
    root = ET.parse(path).getroot()
    counter = count_elements(path)

    stats = {
        "lines": counter.get("line", 0),
        "polylines": counter.get("polyline", 0),
        "rects": counter.get("rect", 0),
        "texts": counter.get("text", 0),
        "circles": counter.get("circle", 0),
        "width": root.get("width"),
        "height": root.get("height"),
        "viewBox": root.get("viewBox"),
    }

    # 分组计数
    kinds: dict[str, int] = {}
    for elem in root.iter():
        if _tag(elem) == "g" and "mark-point" in (elem.get("class") or ""):
            k = elem.get("data-kind")
            kinds[k] = kinds.get(k, 0) + 1
    stats["marks"] = kinds

    # 网格分组元素数
    grid: dict[str, int] = {}
    for elem in root.iter():
        cls = elem.get("class") or ""
        if cls in ("grid-horizontal", "grid-vertical", "palace"):
            grid[cls] = sum(1 for c in elem if _tag(c) == "line")
    stats["grid_groups"] = grid
    return stats


class Checker:
    def __init__(self) -> None:
        self.passed = 0
        self.failed: list[str] = []

    def check(self, label: str, actual, expected) -> None:
        ok = actual == expected
        if ok:
            self.passed += 1
        else:
            self.failed.append(f"{label}: 期望 {expected}，实际 {actual}")
        print(f"  [{'PASS' if ok else 'FAIL'}] {label} = {actual}  (期望 {expected})")

    def report(self) -> bool:
        print()
        print(f"  断言总计: {self.passed + len(self.failed)} 条，"
              f"通过 {self.passed} 条，失败 {len(self.failed)} 条")
        for msg in self.failed:
            print(f"    !! {msg}")
        return not self.failed


# --------------------------------------------------------------------------
# main
# --------------------------------------------------------------------------


def main() -> int:
    print("=" * 68)
    print("弈道 · chinese-chess-rs 视觉素材生成")
    print(f"输出根目录: {ASSETS_DIR}")
    print("=" * 68)

    # ---- 1. 生成 ----
    print("\n[1/4] 生成文件 ...")
    write_text("assets/board/board-classic.svg", build_board_classic())
    write_text("assets/board/board-coords.svg", build_board_coords())
    for name, char, faction in PIECES:
        write_text(f"assets/pieces/{name}.svg", build_piece(name, char, faction))
    write_text("assets/tokens/design-tokens.json", build_tokens())
    write_text("assets/tokens/colors.md", build_colors_md())
    print(f"  共写出 {len(WRITTEN)} 个文件")

    # ---- 2. XML 合法性 ----
    print("\n[2/4] XML 合法性校验（xml.etree.ElementTree.parse）...")
    svg_files = [p for p, _, _ in WRITTEN if p.endswith(".svg")]
    for rel in svg_files:
        ET.parse(os.path.join(ROOT_DIR, rel))
    print(f"  {len(svg_files)} 个 SVG 全部解析通过，无解析异常")
    json.loads(open(os.path.join(ROOT_DIR, "assets/tokens/design-tokens.json"),
                    encoding="utf-8").read())
    print("  design-tokens.json 解析通过")

    # ---- 3. 几何断言 ----
    print("\n[3/4] 几何断言校验 ...")
    ck = Checker()

    print("  -- 数据模型 --")
    ck.check("棋盘横线数量", len(H_LINES), 10)
    ck.check("棋盘竖线段数（含河界断开）", len(V_LINES), 16)
    ck.check("九宫斜线数量", len(PALACE_DIAGONALS), 4)
    ck.check("兵位标记点数", len(SOLDIER_POINTS), 10)
    ck.check("炮位标记点数", len(CANNON_POINTS), 4)
    ck.check("折角总数（边线点仅画内侧）", TOTAL_CORNERS, 48)
    full_span = [seg for seg in V_LINES if seg == (seg[0], Y0, seg[0], Y1)]
    ck.check("竖线完整段数（col0 + col8）", len(full_span), 2)
    ck.check("竖线河界断开段数", len(V_LINES) - len(full_span), 14)
    ck.check("边线兵位点折角数（col=0/8 仅内侧，应为 2 角）",
             sorted({len(g[3]) for g in MARK_GROUPS
                     if g[0] == "soldier" and g[1] in (0, 8)}), [2])
    ck.check("内部兵炮位点折角数（应为 4 角）",
             sorted({len(g[3]) for g in MARK_GROUPS
                     if g[1] not in (0, 8)}), [4])

    # 折角必须是「先水平、再竖直」的直角折线（防止退化成斜线）
    all_corners = [c for g in MARK_GROUPS for c in g[3]]
    elbow_ok = all(
        p[0][1] == p[1][1]           # 第一段水平
        and p[1][0] == p[2][0]       # 第二段竖直
        and abs(p[1][0] - p[0][0]) == MARK_LEN
        and abs(p[2][1] - p[1][1]) == MARK_LEN
        for p in all_corners
    )
    ck.check("折角直角/长度合法（水平 8px + 竖直 8px）", elbow_ok, True)
    ck.check("折角不入河界/不越框", all(
        X0 <= min(x for x, _ in p) and max(x for x, _ in p) <= X1
        and Y0 <= min(y for _, y in p) and max(y for _, y in p) <= Y1
        for p in all_corners), True)

    print("  -- 落盘 SVG（解析后统计）--")
    classic = verify_svg(os.path.join(ROOT_DIR, "assets/board/board-classic.svg"))
    ck.check("board-classic viewBox", classic["viewBox"], "0 0 560 620")
    ck.check("board-classic 横线 <line> 数", classic["grid_groups"]["grid-horizontal"], 10)
    ck.check("board-classic 竖线 <line> 数", classic["grid_groups"]["grid-vertical"], 16)
    ck.check("board-classic 九宫 <line> 数", classic["grid_groups"]["palace"], 4)
    ck.check("board-classic <line> 总数", classic["lines"], 30)
    ck.check("board-classic 折角 <polyline> 数", classic["polylines"], 48)
    ck.check("board-classic 兵位标记组数", classic["marks"].get("soldier"), 10)
    ck.check("board-classic 炮位标记组数", classic["marks"].get("cannon"), 4)
    ck.check("board-classic <text> 数（楚河 + 汉界）", classic["texts"], 2)

    coords = verify_svg(os.path.join(ROOT_DIR, "assets/board/board-coords.svg"))
    ck.check("board-coords viewBox", coords["viewBox"], "0 0 590 630")
    ck.check("board-coords <text> 数（2 + 9 列 + 10 行）", coords["texts"], 21)
    ck.check("board-coords 折角 <polyline> 数", coords["polylines"], 48)

    piece_root = os.path.join(ROOT_DIR, "assets/pieces")
    piece_bad = []
    for name, char, faction in PIECES:
        st = verify_svg(os.path.join(piece_root, f"{name}.svg"))
        if st["viewBox"] != "0 0 64 64" or st["circles"] != 4 or st["polylines"] != 0:
            piece_bad.append(name)
    ck.check("14 个棋子 viewBox/同心圆结构异常数", len(piece_bad), 0)
    ck.check("棋子文件数", len(PIECES), 14)

    ok = ck.report()

    # ---- 4. 清单 ----
    print("\n[4/4] 产物清单（文件名 + 字节数 + sha256 前缀）")
    print(f"  {'文件':<44}{'字节数':>9}  sha256")
    print("  " + "-" * 74)
    total = 0
    for rel, size, digest in sorted(WRITTEN):
        total += size
        print(f"  {rel:<44}{size:>9}  {digest[:12]}")
    print("  " + "-" * 74)
    print(f"  {'合计 ' + str(len(WRITTEN)) + ' 个文件':<44}{total:>9}")

    # 组合校验总指纹（用于幂等性比对）
    combined = hashlib.sha256()
    for rel, _, digest in sorted(WRITTEN):
        combined.update(rel.encode("utf-8"))
        combined.update(digest.encode("ascii"))
    print(f"\n  组合指纹 (combined-sha256): {combined.hexdigest()}")

    print("\n" + "=" * 68)
    print("结果: " + ("全部通过 ✅" if ok else "存在失败断言 ❌"))
    print("=" * 68)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
