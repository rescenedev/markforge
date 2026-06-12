#!/usr/bin/env python3
"""Generate MarkForge's app icon (a Markdown 'M↓' mark on a blue squircle).

The palette matches the landing page (Toss-blue accent). Drawn at 4x and
downsampled for clean antialiasing. Produces `assets/icon.png` (1024px) and a
macOS `.iconset` folder. Run via `scripts/make_icns.sh`, which then calls
`iconutil` to build `assets/icon.icns`.
"""
import os

from PIL import Image, ImageDraw, ImageFilter

SIZE = 1024
SS = 4  # supersampling factor
S = SIZE * SS
HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT_PNG = os.path.join(HERE, "assets", "icon.png")
ICONSET = os.path.join(HERE, "assets", "MarkForge.iconset")

Color = tuple[int, int, int]


def lerp(a: Color, b: Color, t: float) -> Color:
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def rounded_mask(size: int, radius: int) -> Image.Image:
    mask = Image.new("L", (size, size), 0)
    d = ImageDraw.Draw(mask)
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, fill=255)
    return mask


def gradient_base() -> Image.Image:
    """Blue vertical gradient with a faint top glow, masked to a squircle."""
    top = (77, 162, 255)     # #4DA2FF
    bottom = (27, 100, 218)  # #1B64DA
    grad = Image.new("RGB", (S, S))
    px = grad.load()
    for y in range(S):
        c = lerp(top, bottom, y / (S - 1))
        for x in range(S):
            px[x, y] = c

    # Subtle radial highlight near the top so the surface reads as curved.
    glow = Image.new("L", (S, S), 0)
    d = ImageDraw.Draw(glow)
    d.ellipse([-S * 0.35, -S * 0.75, S * 1.35, S * 0.55], fill=46)
    glow = glow.filter(ImageFilter.GaussianBlur(S * 0.05))
    grad.paste(Image.new("RGB", (S, S), (255, 255, 255)), (0, 0), glow)

    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    img.paste(grad, (0, 0), rounded_mask(S, int(S * 0.2237)))
    return img


def glyph_layer() -> Image.Image:
    """The 'M↓' mark as a grayscale layer (used for both shadow and fill)."""
    layer = Image.new("L", (S, S), 0)
    d = ImageDraw.Draw(layer)
    w = 82 * SS  # stroke width

    def dot(p: tuple[int, int]) -> None:
        d.ellipse(
            [p[0] - w // 2, p[1] - w // 2, p[0] + w // 2, p[1] + w // 2],
            fill=255,
        )

    # "M" — two outer verticals + an inner V.
    m = [
        (290 * SS, 688 * SS),
        (290 * SS, 352 * SS),
        (438 * SS, 540 * SS),
        (586 * SS, 352 * SS),
        (586 * SS, 688 * SS),
    ]
    d.line(m, fill=255, width=w, joint="curve")
    for p in m:
        dot(p)

    # Down arrow (the Markdown mark's descent).
    stem_x = 732 * SS
    d.line([(stem_x, 352 * SS), (stem_x, 556 * SS)], fill=255, width=w)
    dot((stem_x, 352 * SS))
    d.polygon(
        [
            (stem_x - 102 * SS, 540 * SS),
            (stem_x + 102 * SS, 540 * SS),
            (stem_x, 700 * SS),
        ],
        fill=255,
    )
    return layer


def compose() -> Image.Image:
    img = gradient_base()
    glyph = glyph_layer()

    # Soft drop shadow for a little depth, clipped to the squircle.
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    shadow_alpha = glyph.filter(ImageFilter.GaussianBlur(10 * SS)).point(
        lambda a: a * 30 // 100
    )
    shadow.paste((8, 36, 90, 255), (0, 0), shadow_alpha)
    img.alpha_composite(shadow, (0, 14 * SS))

    white = Image.new("RGBA", (S, S), (255, 255, 255, 255))
    img.paste(white, (0, 0), glyph)

    return img.resize((SIZE, SIZE), Image.LANCZOS)


def main() -> None:
    os.makedirs(os.path.dirname(OUT_PNG), exist_ok=True)
    base = compose()
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
