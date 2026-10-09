#!/usr/bin/env python3
"""キャラ・アイテム用の 8x8 スプライトシート (assets/sprites.png) を生成する。

すべてこのスクリプトで描いたオリジナル。実行すると assets/sprites.png を上書きする。
    python3 tools/gen_sprites.py

並び（8列x1行、左から）:
    0 robot_idle   1 robot_run   2 robot_jump   3 robot_si（SI 形態）
    4 boss         5 boss_stunned（看板が AI に戻って目を回している）
    6 mushroom（Sキノコ）  7 goal（旗）
"""
from pathlib import Path

from PIL import Image

PALETTE = {
    ".": None,
    "k": (0, 0, 0),
    "w": (252, 252, 252),
    "g": (188, 188, 188),  # 明るい灰色
    "G": (116, 116, 116),  # 暗い灰色
    "b": (0, 88, 248),  # 青
    "c": (60, 188, 252),  # 水色
    "r": (216, 40, 0),  # 赤
    "y": (252, 184, 0),  # 黄
    "n": (252, 188, 176),  # 肌色
    "l": (0, 168, 0),  # 緑
    "s": (92, 60, 24),  # 茶
}

# 8x8 ×、1 行 8 文字
SPRITES = {
    "robot_idle": [
        "...yy...",
        "..gggg..",
        ".gcwwcg.",
        ".gckkcg.",
        "..gggg..",
        ".GggggG.",
        ".G.gg.G.",
        "..kk.kk.",
    ],
    "robot_run": [
        "...yy...",
        "..gggg..",
        ".gcwwcg.",
        ".gckkcg.",
        "..gggg..",
        "GgggggG.",
        "..g..gG.",
        ".kk...kk",
    ],
    "robot_jump": [
        "...yy...",
        "..gggg..",
        ".gcwwcg.",
        ".gckkcg.",
        "G.gggg.G",
        "GGggggGG",
        "..g..g..",
        ".kk..kk.",
    ],
    "robot_si": [
        "...yy...",
        "..gggg..",
        ".gcwwcg.",
        ".gckkcg.",
        "..gggg..",
        ".GgyygG.",
        ".G.yy.G.",
        "..kk.kk.",
    ],
    "boss": [
        "..kkkk..",
        ".nnnnnn.",
        ".nknnkn.",
        "..nnnn..",
        ".GGwrGG.",
        "GGGwrGGG",
        ".GG..GG.",
        ".kk..kk.",
    ],
    "boss_stunned": [
        "..kkkk..",
        ".nnnnnn.",
        ".nwnnwn.",
        "..nkkn..",
        ".GGwrGG.",
        "GGGwrGGG",
        ".GG..GG.",
        ".kk..kk.",
    ],
    "mushroom": [
        ".rwwwwr.",
        ".rwrrrr.",
        ".rwwwwr.",
        ".rrrrwr.",
        ".rwwwwr.",
        "..nwwn..",
        "..nnnn..",
        "...nn...",
    ],
    "goal": [
        ".glllll.",
        ".gllllll",
        ".glllll.",
        ".g......",
        ".g......",
        ".g......",
        ".g......",
        "gggg....",
    ],
}


def main() -> None:
    order = list(SPRITES.keys())
    sheet = Image.new("RGBA", (8 * len(order), 8), (0, 0, 0, 0))
    for i, name in enumerate(order):
        rows = SPRITES[name]
        assert len(rows) == 8, name
        for y, row in enumerate(rows):
            assert len(row) == 8, f"{name} row {y}: {row!r}"
            for x, ch in enumerate(row):
                rgb = PALETTE[ch]
                if rgb is not None:
                    sheet.putpixel((i * 8 + x, y), (*rgb, 255))
    out = Path(__file__).resolve().parent.parent / "assets" / "sprites.png"
    out.parent.mkdir(exist_ok=True)
    sheet.save(out)
    print(f"wrote {out} ({sheet.width}x{sheet.height}), sprites: {order}")


if __name__ == "__main__":
    main()
