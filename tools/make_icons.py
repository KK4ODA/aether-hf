"""The application's icons and the panel's brand images, from the artwork.

The source files are `Logos/Aether HF Icon.png` (a rounded square on black) and
`Logos/Aether HF Logo.png` (the wordmark on black). The icon's black surround is cut
away with a rounded-rectangle mask measured from the artwork, so the desktop shows the tile
and not a black square around it; the banner is only resized, since the panel's splash is
black behind it anyway.

    python tools/make_icons.py

Needs Pillow (`python -m uv run --with pillow python tools/make_icons.py` works without
installing it).
"""

from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[1]
ARTWORK = ROOT / "Logos"
ICONS = ROOT / "app" / "src-tauri" / "icons"
UI = ROOT / "app" / "ui"

# the tile's rounded rectangle in the 1254 px source, measured from where the glow ends
TILE = (31, 26, 1222, 1210)
TILE_RADIUS = 265


def tile() -> Image.Image:
    source = Image.open(ARTWORK / "Aether HF Icon.png").convert("RGB")
    mask = Image.new("L", source.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle(TILE, radius=TILE_RADIUS, fill=255)
    mask = mask.filter(ImageFilter.GaussianBlur(1.2))
    rgba = source.convert("RGBA")
    rgba.putalpha(mask)
    left, top, right, bottom = TILE
    cropped = rgba.crop((left - 3, top - 3, right + 4, bottom + 4))
    side = max(cropped.size)
    square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    square.paste(cropped, ((side - cropped.width) // 2, (side - cropped.height) // 2))
    return square


def main() -> None:
    square = tile()
    resample = Image.LANCZOS
    square.resize((32, 32), resample).save(ICONS / "32x32.png")
    square.resize((128, 128), resample).save(ICONS / "128x128.png")
    square.resize((256, 256), resample).save(ICONS / "128x128@2x.png")
    square.resize((512, 512), resample).save(ICONS / "icon.png")
    square.resize((256, 256), resample).save(
        ICONS / "icon.ico",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    square.resize((96, 96), resample).save(UI / "mark.png")
    square.resize((32, 32), resample).save(UI / "favicon.png")
    logo = Image.open(ARTWORK / "Aether HF Logo.png").convert("RGB")
    width = 1000
    logo.resize((width, round(width * logo.height / logo.width)), resample).save(
        UI / "logo.png", optimize=True
    )
    print("icons, mark, favicon and logo written")


if __name__ == "__main__":
    main()
