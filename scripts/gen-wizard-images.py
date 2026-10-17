#!/usr/bin/env python3
"""Generate the Inno Setup wizard bitmaps for installer/assets/.

The wizard side image MUST share the display area's aspect ratio (164:314)
or Inno letterboxes it; render at 3x the 100%-DPI area (606x1158) so every
DPI tier downsamples cleanly. The small image must be square (rendered
240x240). Sources: docs/assets/logo.png (brand shield) + a dark vertical
gradient + wordmark. 24-bit BMPs (Inno's wizard slots; no color-key needed
since the canvas is fully painted).
"""
from PIL import Image, ImageDraw, ImageFilter, ImageFont
import pathlib, sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
ASSETS = ROOT / "installer" / "assets"
LOGO = ROOT / "docs" / "assets" / "logo.png"

# palette (from logo.svg brand colors)
TOP = (2, 18, 44)      # #00122C
BOT = (6, 30, 60)      # deep navy, slight lift
ACCENT = (40, 100, 199)  # #2864C7
TEXT = (235, 240, 248)
DIM = (150, 170, 200)

SCALE = 3
W, H = 202 * SCALE, 386 * SCALE  # 606x1158


def font(size):
    for path in (
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    ):
        try:
            return ImageFont.truetype(path, size)
        except OSError:
            continue
    return ImageFont.load_default()


def font_bold(size):
    try:
        return ImageFont.truetype(
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", size)
    except OSError:
        return font(size)


def lerp(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def render_big():
    img = Image.new("RGB", (W, H))
    d = ImageDraw.Draw(img)
    # vertical gradient with a faint diagonal glow behind the logo
    for y in range(H):
        d.line([(0, y), (W, y)], fill=lerp(TOP, BOT, y / H))
    glow = Image.new("L", (W, H), 0)
    gd = ImageDraw.Draw(glow)
    gd.ellipse((W * 0.18, H * 0.16, W * 0.82, H * 0.50), fill=70)
    glow = glow.filter(ImageFilter.GaussianBlur(W * 0.18))
    img = Image.composite(Image.new("RGB", (W, H), lerp(TOP, ACCENT, 0.55)), img, glow)
    d = ImageDraw.Draw(img)

    # shield logo, large, horizontally centered
    logo = Image.open(LOGO).convert("RGBA")
    side = int(W * 0.62)
    logo = logo.resize((side, side), Image.LANCZOS)
    lx, ly = (W - side) // 2, int(H * 0.16)
    img.paste(logo, (lx, ly), logo)

    # wordmark
    wf = font_bold(int(W * 0.16))
    d.text((W // 2, int(H * 0.585)), "Rism", font=wf, fill=TEXT, anchor="mm")
    tag = "MCP server + CLI for InterSystems IRIS"
    size = int(W * 0.052)
    tf = font(size)
    while size > 8 and d.textlength(tag, font=tf) > W * 0.86:
        size -= 1  # shrink until the tagline fits with clear side margins
        tf = font(size)
    d.text((W // 2, int(H * 0.645)), tag, font=tf, fill=DIM, anchor="mm")

    # thin accent rule
    d.rectangle((int(W * 0.24), int(H * 0.70), int(W * 0.76), int(H * 0.70) + SCALE),
                fill=lerp(ACCENT, TEXT, 0.25))

    # terminal-flavored footer block
    mono = font(int(W * 0.052))
    lines = ["$ rism doctor", "$ rism mcp", "$ rism sql \"select 1\""]
    y0 = int(H * 0.755)
    for i, ln in enumerate(lines):
        d.text((int(W * 0.12), y0 + i * int(H * 0.035)), ln, font=mono,
               fill=lerp(DIM, TEXT, 0.35))
    return img


def render_small():
    S = 240
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    logo = Image.open(LOGO).convert("RGBA")
    logo = logo.resize((S, S), Image.LANCZOS)
    return logo


def main():
    if "--check" in sys.argv:
        for f in ("wizard-image.bmp", "wizard-small.bmp"):
            p = ASSETS / f
            if not p.exists():
                print(f"MISSING {p}")
                sys.exit(1)
            print(f"OK {p.name} {Image.open(p).size}")
        return
    render_big().convert("RGB").save(ASSETS / "wizard-image.bmp")
    render_small().save(ASSETS / "wizard-small.png")
    print("wrote", ASSETS / "wizard-image.bmp", ASSETS / "wizard-small.png")


if __name__ == "__main__":
    main()
