"""生成棋局语音素材（关键词 + 走法播报片段）。

# 为什么素材要生成而不是手放

要喊的话分成两类：固定词（吃车、将军、绝杀、45 个战术名）和走法播报的片段
（「炮二」「平五」）。固定词逐条录；播报片靠有限片段拼装 —— 棋子 9 个 ×
纵线 9 个 + 动作 3 个 × 数 9 个，一百来条就能覆盖任意一步棋的记谱。

# 合成用文本 ≠ 显示用文本

多音字必须换掉：合成器会按最常见的读法念（车念 chē、重念 zhòng、卒念 cù），
而象棋里读 jū / chóng / zú。词表里的 `aliases` 就是干这个的 —— 它只影响送去
合成的那串字，界面显示的永远是棋盘上的真词。详见 vocabulary.json 的说明。

# 用法

    python generate.py                # 缺什么补什么（可反复跑）
    python generate.py --force        # 全部重生成
    python generate.py --list         # 只列出词表和文件名，不合成
"""

from __future__ import annotations

import argparse
import asyncio
import json
import pathlib
import sys

import edge_tts

ROOT = pathlib.Path(__file__).resolve().parents[3]
VOICE_DIR = ROOT / "assets" / "voice"
VOCABULARY = pathlib.Path(__file__).with_name("vocabulary.json")

# 动作字用拼音当 id 的一部分，id 里不混中文
ACTION_SLUG = {"进": "jin", "退": "tui", "平": "ping"}

CONCURRENCY = 3
RETRIES = 4


def apply_aliases(text: str, aliases: dict[str, str]) -> str:
    """按「长键优先」替换，否则「重炮」会被「炮」之类的短键先咬掉。"""
    for source in sorted(aliases, key=len, reverse=True):
        text = text.replace(source, aliases[source])
    return text


def walk_tactics(node: object, found: dict[str, str]) -> dict[str, str]:
    """从 tactics.json 里挖出 {id: 中文名}。结构是嵌套的，所以用递归找。"""
    if isinstance(node, dict):
        if "id" in node and "name" in node:
            found[node["id"]] = node["name"]
            return found
        for value in node.values():
            walk_tactics(value, found)
    elif isinstance(node, list):
        for value in node:
            walk_tactics(value, found)
    return found


def build_clips(vocabulary: dict) -> list[dict]:
    """把词表展开成一条条要合成的素材。"""
    aliases = vocabulary["aliases"]
    clips: list[dict] = []

    def add(clip_id: str, text: str, synth: str | None = None) -> None:
        clips.append(
            {
                "id": clip_id,
                "text": text,
                "synth": synth if synth is not None else apply_aliases(text, aliases),
            }
        )

    for entry in vocabulary["clips"]:
        add(entry["id"], entry["text"], entry.get("synth"))

    numerals = vocabulary["numerals"]
    numeral_value = {numeral: index for index, numeral in enumerate(numerals, start=1)}

    for piece in vocabulary["pieces"]:
        base = piece.get("synth", apply_aliases(piece["text"], aliases))
        # 文件名用阿拉伯数字：id 要能在命令行、URL、日志里干干净净地传
        for index, numeral in enumerate(numerals, start=1):
            add(f"word-{piece['id']}-{index}", piece["text"] + numeral, base + numeral)

    # 「前车」「中炮」这类：序数在前、棋子字在后，所以和 word-* 是两种形状
    for qualifier in vocabulary["qualifiers"]:
        for piece in vocabulary["pieces"]:
            base = piece.get("synth", apply_aliases(piece["text"], aliases))
            add(
                f"qual-{piece['id']}-{qualifier['id']}",
                qualifier["text"] + piece["text"],
                qualifier["text"] + base,
            )

    by_id = {piece["id"]: piece for piece in vocabulary["pieces"]}
    for piece_id in vocabulary["nth_pieces"]:
        piece = by_id[piece_id]
        base = piece.get("synth", apply_aliases(piece["text"], aliases))
        for numeral in vocabulary["nth_numerals"]:
            add(
                f"nth-{piece_id}-{numeral_value[numeral]}",
                numeral + piece["text"],
                numeral + base,
            )

    for action in vocabulary["actions"]:
        for index, numeral in enumerate(numerals, start=1):
            add(f"move-{ACTION_SLUG[action]}-{index}", action + numeral, action + numeral)

    source = ROOT / vocabulary["tactic_source"]
    tactics = walk_tactics(json.loads(source.read_text(encoding="utf-8")), {})
    for tactic_id, name in sorted(tactics.items()):
        add(f"tactic-{tactic_id}", name)

    return clips


def assign_files(clips: list[dict]) -> dict[str, str]:
    """同一个合成文本只生成一个文件，多个 id 共用 —— 「将军」既是状态关键词
    也是战术名，没必要存两份。文件名取第一个用它的 id。"""
    by_synth: dict[str, str] = {}
    for clip in clips:
        by_synth.setdefault(clip["synth"], clip["id"])
    return {clip["id"]: f"{by_synth[clip['synth']]}.mp3" for clip in clips}


def is_done(path: pathlib.Path) -> bool:
    """空文件算没生成。

    上次跑到一半超时会留下 0 字节的文件；只看「文件在不在」的话，这些残骸会被
    当成成品永远跳过 —— 素材缺一块，而且是静默缺的。
    """
    return path.exists() and path.stat().st_size > 0


async def render_one(synth: str, path: pathlib.Path, voice: str, rate: str,
                     semaphore: asyncio.Semaphore, force: bool) -> str:
    if is_done(path) and not force:
        return "skip"
    # 合成走的是远程服务，偶发超时是常态 —— 重试几次，别让一次抖动废掉整批
    async with semaphore:
        for attempt in range(1, RETRIES + 1):
            try:
                await edge_tts.Communicate(synth, voice, rate=rate).save(str(path))
                return "made"
            except Exception as error:  # noqa: BLE001 —— 网络异常种类多，统一重试
                if attempt == RETRIES:
                    print(f"  ✗ {path.name}（{synth}）放弃：{error}", file=sys.stderr)
                    return "failed"
                await asyncio.sleep(attempt * 2)
    return "failed"


def unique_renders(clips: list[dict], files: dict[str, str]) -> dict[str, pathlib.Path]:
    """按**去重后的文件**生成，而不是按 id。

    多个 id 可能共用同一个文件（相/象、仕/士 同音，将军既是状态关键词又是战术名）。
    按 id 发任务的话，两个任务会同时写同一个文件 —— 谁后写完谁赢，中间还有交错
    写入把文件写坏的可能。
    """
    renders: dict[str, pathlib.Path] = {}
    for clip in clips:
        renders.setdefault(clip["synth"], VOICE_DIR / files[clip["id"]])
    return renders


async def main() -> int:
    parser = argparse.ArgumentParser(description="生成棋局语音素材")
    parser.add_argument("--force", action="store_true", help="已有文件也重新生成")
    parser.add_argument("--prune", action="store_true", help="删掉不属于当前词表的残留 mp3")
    parser.add_argument("--list", action="store_true", help="只列词表，不合成")
    parser.add_argument("--only", metavar="前缀", help="只处理 id 以该前缀开头的")
    args = parser.parse_args()

    vocabulary = json.loads(VOCABULARY.read_text(encoding="utf-8"))
    all_clips = build_clips(vocabulary)
    all_files = assign_files(all_clips)
    clips = [c for c in all_clips if c["id"].startswith(args.only)] if args.only else all_clips
    if not clips:
        print("没有匹配的素材", file=sys.stderr)
        return 1

    voice, rate = vocabulary["voice"], vocabulary["rate"]

    if args.list:
        for clip in clips:
            note = "" if clip["synth"] == clip["text"] else f"   ← 合成念「{clip['synth']}」"
            print(f"  {all_files[clip['id']]:32s} {clip['text']}{note}")
        print(f"\n共 {len(clips)} 个 id，去重后 {len(unique_renders(all_clips, all_files))} 个文件")
        return 0

    VOICE_DIR.mkdir(parents=True, exist_ok=True)
    renders = unique_renders(all_clips, all_files)
    semaphore = asyncio.Semaphore(CONCURRENCY)
    tasks = [
        render_one(synth, path, voice, rate, semaphore, args.force)
        for synth, path in renders.items()
    ]
    results = await asyncio.gather(*tasks)
    made = results.count("made")

    manifest = {
        "voice": voice,
        "rate": rate,
        "generated_from": "scripts/dev/voice/vocabulary.json",
        "note": "text 是界面上显示的真词，synth 是实际送去合成的文本（多音字已换同音字）。file 相对 /voice/ 目录。",
        "clips": {
            clip["id"]: {"file": all_files[clip["id"]], "text": clip["text"], "synth": clip["synth"]}
            for clip in all_clips
        },
    }
    (VOICE_DIR / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )

    on_disk = {f for f in VOICE_DIR.glob("*.mp3")}
    expected = {path for path in renders.values()}
    missing = sorted(p.name for p in expected if not is_done(p))
    extra = sorted(p.name for p in on_disk - expected)

    size = sum(p.stat().st_size for p in on_disk)
    print(f"新生成 {made} 个，跳过 {len(results) - made} 个")
    print(f"目录内 {len(on_disk)} 个 mp3，合计 {size / 1024:.0f} KB（清单 {len(all_clips)} 个 id）")
    print(f"清单：{(VOICE_DIR / 'manifest.json').relative_to(ROOT)}")

    if args.prune and extra:
        for name in extra:
            (VOICE_DIR / name).unlink()
        print(f"清掉 {len(extra)} 个不属于当前词表的残留文件")
        extra = []
    elif extra:
        print(f"⚠ 目录里另有 {len(extra)} 个不属于当前词表的文件（多半是改了命名留下的残骸）")
        print("  加 --prune 清掉")

    if missing:
        print(f"✗ 还有 {len(missing)} 个没生成：{'、'.join(missing[:6])}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
