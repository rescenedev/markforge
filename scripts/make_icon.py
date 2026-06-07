#!/usr/bin/env python3
"""Generate MarkForge's app icon (a Markdown 'M↓' mark on a gradient squircle).

Produces `assets/icon.png` (1024px) and a macOS `.iconset` folder. Run via
`scripts/make_icns.sh`, which then calls `iconutil` to build `assets/icon.icns`.
"""
import os
from PIL import Image, ImageDraw

SIZE = 1024
HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT_PNG = os.path.join(HERE, "assets", "icon.png")
ICONSET = os.path.join(HERE, "assets", "MarkForge.iconset")


def lerp(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def rounded_mask(size, radius):
    mask = Image.new("L", (size, size), 0)
    d = ImageDraw.Draw(mask)
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, fill=255)
    return mask


def make_base():
    # Vertical gradient: indigo -> violet.
    top = (99, 102, 241)     # #6366F1
    bottom = (139, 92, 246)  # #8B5CF6
    grad = Image.new("RGB", (SIZE, SIZE))
    px = grad.load()
    for y in range(SIZE):
        c = lerp(top, bottom, y / (SIZE - 1))
        for x in range(SIZE):
            px[x, y] = c

    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    img.paste(grad, (0, 0), rounded_mask(SIZE, int(SIZE * 0.2237)))

    d = ImageDraw.Draw(img)
    white = (255, 255, 255, 255)
    w = 78  # stroke width

    # "M" — two outer verticals + an inner V.
    m = [(296, 690), (296, 356), (440, 536), (584, 356), (584, 690)]
    d.line(m, fill=white, width=w, joint="curve")
    for p in m:  # round the caps/joints
        d.ellipse([p[0] - w // 2, p[1] - w // 2, p[0] + w // 2, p[1] + w // 2], fill=white)

    # Down arrow (the Markdown mark's descent).
    stem_x = 726
    d.line([(stem_x, 356), (stem_x, 560)], fill=white, width=w, joint="curve")
    d.ellipse([stem_x - w // 2, 356 - w // 2, stem_x + w // 2, 356 + w // 2], fill=white)
    d.polygon([(stem_x - 96, 544), (stem_x + 96, 544), (stem_x, 700)], fill=white)

    return img


def main():
    os.makedirs(os.path.dirname(OUT_PNG), exist_ok=True)
    base = make_base()
    base.save(OUT_PNG)

    os.makedirs(ICONSET, exist_ok=True)
    specs = [
        (16, 1), (16, 2), (32, 1), (32, 2), (128, 1), (128, 2),
        (256, 1), (256, 2), (512, 1), (512, 2),
    ]
    for base_px, scale in specs:
        px = base_px * scale
        name = f"icon_{base_px}x{base_px}{'' if scale == 1 else '@2x'}.png"
        base.resize((px, px), Image.LANCZOS).save(os.path.join(ICONSET, name))

    print("wrote", OUT_PNG, "and", ICONSET)


if __name__ == "__main__":
    main()
