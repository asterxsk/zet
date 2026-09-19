#!/usr/bin/env python3
"""Draw `zet.ico`, the application icon, and the preview sheet used to judge it.

The mark is the one the tab strip uses: a `#`, which DESIGN.md calls "part of the
mark, not a prefix that disappears". Centred in its counter is a small square lit in
`signal` — a lit pixel inside the number, which is the same thing the chrome does with
the same colour: one lamp saying which of several things is active.

The amber is deliberately small. Filling the counter edge to edge was the first draft
and it read as a photograph, not a terminal: the eye takes the largest saturated area
as the subject, so a filled counter makes the `#` a frame around an orange square.
At half the counter the strokes are the subject and the amber is what it should be —
a lamp. That is also why the strokes thicken below 48px and the counter does not: at
16px a hairline `#` greys out, and the pixel it encloses has to survive the trip.

Geometry is specified once, in a 256-unit box, and every size is rendered natively
from it at 8x and reduced, rather than resampled from one large bitmap. That is what
keeps the strokes from turning to grey mush in the sizes the taskbar actually uses.

    python packaging/make-icon.py            # writes zet.ico and zet.svg beside this file
    python packaging/make-icon.py --preview  # also writes a sheet of every size
    python packaging/make-icon.py --docs docs  # also writes the docs site's icon set

The SVG is emitted from the same numbers as the bitmap rather than drawn by hand, so
the two cannot drift into being different marks. It is there for the places a `.ico`
is no use — the documentation site, the README, anything that wants a scalable copy.

`--docs` writes the four files the documentation site needs: a favicon, that same SVG,
an opaque 180px touch icon, and the social card. They live in `docs/` and are committed,
because GitHub Pages publishes that directory exactly as it is, with no build step to
generate anything on the way out.

The preview sheet goes to `D:\\Apps\\tmp\\zet\\` — never into the repository, because
the only reason to look at it is to decide whether to change the numbers below.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw
except ImportError:  # pragma: no cover - the message is the point
    sys.exit("this script needs Pillow: python -m pip install pillow")

# The unit box. Every number below is a fraction of this, so the sheet can be drawn at
# any size without the mark changing shape.
UNIT = 256.0

# The chrome palette, from DESIGN.md. Hard-coded rather than imported because this
# script has to run without the workspace, and because a palette edit that silently
# repainted the icon would be a surprise.
GROUND = (0x0A, 0x0B, 0x0D, 255)
INK = (0xE7, 0xE9, 0xEC, 255)
SIGNAL = (0xFF, 0xA6, 0x2B, 255)

# --- the mark, in unit coordinates -------------------------------------------------

# The rounded tile the mark sits on. The radius is generous enough to read as a modern
# app tile without turning the `#` into a circle at 16px.
TILE_RADIUS = 58.0

# Where the two vertical strokes sit, and the two horizontal ones. The distance
# between them minus the stroke width is the counter, so this number is also what
# decides how much amber there is.
STROKE_CENTRES = (86.0, 169.0)

# How thick the strokes are, before the optical correction below.
STROKE = 24.0

# How far the strokes run past the outer edge of the crossing stroke. At about three
# quarters of the stroke width this is what a `#` looks like; much more and the mark
# stops being a `#` and becomes a fence, which is what the first draft of this file
# drew. Measured from the edge, not the centre line, so it stays honest if STROKE
# changes.
OVERSHOOT = 20.0

# The stroke is a fixed fraction of the box at large sizes and thicker at small ones.
#
# This is optical sizing, and it is not a fudge. A stroke that is 24 units of 256 is
# 1.5 pixels at 16px — under the width at which a rendered line holds its colour, so
# the whole mark greys out exactly where it is most often seen. The correction is
# applied only below 48px, above which the geometric weight is what the icon is.
LIGHT_STROKE_FROM = 48

# How much of the counter the lit pixel fills, as a fraction of its width.
LIT = 0.5


def stroke_for(size: int) -> float:
    """The stroke width, in unit coordinates, that this size needs."""
    if size >= LIGHT_STROKE_FROM:
        return STROKE
    # 16px wants about 2.1 pixels; 32px wants 3.5. Interpolating on the pixel width
    # rather than the size keeps the two ends honest.
    wanted_pixels = 2.1 + (size - 16) * (3.5 - 2.1) / (32 - 16)
    return UNIT * wanted_pixels / size

SIZES = (16, 24, 32, 48, 64, 128, 256)

# Rendering each size at this multiple and reducing is the whole reason the 16px icon
# has clean edges. Pillow has no antialiasing of its own for shapes.
SUPERSAMPLE = 8

TMP = Path(r"D:\Apps\tmp\zet")


def draw_mark(size: int, radius: float = TILE_RADIUS) -> Image.Image:
    """Render the mark at `size` pixels square, with an alpha channel.

    `radius` is the tile's corner radius in unit coordinates. The documentation site's
    touch icon asks for `0`, because iOS masks that icon itself and a rounded tile
    inside a rounded mask reads as a double border.
    """
    scale = size * SUPERSAMPLE / UNIT
    canvas = Image.new("RGBA", (size * SUPERSAMPLE, size * SUPERSAMPLE), (0, 0, 0, 0))
    pen = ImageDraw.Draw(canvas)

    def u(value: float) -> float:
        return value * scale

    pen.rounded_rectangle(
        (0, 0, size * SUPERSAMPLE - 1, size * SUPERSAMPLE - 1),
        radius=u(radius),
        fill=GROUND,
    )

    stroke = stroke_for(size)
    half = stroke / 2.0
    near, far = STROKE_CENTRES
    # The stroke runs from the outer edge of one crossing to the outer edge of the
    # other, plus the overshoot at each end.
    run = (near - half - OVERSHOOT, far + half + OVERSHOOT)

    for centre in STROKE_CENTRES:
        # Vertical stroke.
        pen.rectangle(
            (u(centre - half), u(run[0]), u(centre + half), u(run[1])),
            fill=INK,
        )
        # Horizontal stroke, the same run the other way.
        pen.rectangle(
            (u(run[0]), u(centre - half), u(run[1]), u(centre + half)),
            fill=INK,
        )

    # The lit pixel, centred in the counter. Half the counter's width, so the ink
    # strokes stay the subject and this stays a lamp.
    low, high = near + half, far - half
    middle = (low + high) / 2.0
    reach = (high - low) * LIT / 2.0
    pen.rectangle(
        (u(middle - reach), u(middle - reach), u(middle + reach), u(middle + reach)),
        fill=SIGNAL,
    )

    return canvas.resize((size, size), Image.LANCZOS)


def write_ico(path: Path) -> None:
    """Write the multi-size icon.

    The largest rendering is the source and the rest are drawn from the geometry, not
    from it: Pillow's ICO writer reduces whatever it is handed, and a 16px icon made
    by shrinking a 256px one is exactly the blurry result this script exists to avoid.
    So the largest frame is passed as the image and every other size is rendered here
    and handed over through `append_images`.
    """
    largest = draw_mark(256)
    frames = [draw_mark(size) for size in SIZES if size != 256]
    largest.save(
        path,
        format="ICO",
        sizes=[(size, size) for size in SIZES],
        append_images=frames,
    )


def write_svg(path: Path) -> None:
    """Write the mark as SVG, from the same geometry the bitmap uses."""
    stroke = STROKE
    half = stroke / 2.0
    near, far = STROKE_CENTRES
    low, high = near + half, far - half
    middle = (low + high) / 2.0
    reach = (high - low) * LIT / 2.0
    run_start = near - half - OVERSHOOT
    run_end = far + half + OVERSHOOT
    run_length = run_end - run_start

    def hexed(colour: tuple[int, int, int, int]) -> str:
        return f"#{colour[0]:02x}{colour[1]:02x}{colour[2]:02x}"

    # Vertical strokes run the full height of the run; horizontal ones the full width.
    # Both are drawn from the same two centre lines.
    strokes = []
    for centre in STROKE_CENTRES:
        strokes.append(
            f'  <rect x="{centre - half:g}" y="{run_start:g}" '
            f'width="{stroke:g}" height="{run_length:g}"/>'
        )
        strokes.append(
            f'  <rect x="{run_start:g}" y="{centre - half:g}" '
            f'width="{run_length:g}" height="{stroke:g}"/>'
        )

    path.write_text(
        "\n".join(
            [
                '<?xml version="1.0" encoding="UTF-8"?>',
                '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" '
                'width="256" height="256" role="img" '
                'aria-label="zet">',
                "  <title>zet</title>",
                "  <!-- Generated by make-icon.py. Edit the constants there, not here. -->",
                f'  <rect width="256" height="256" rx="{TILE_RADIUS:g}" '
                f'fill="{hexed(GROUND)}"/>',
                f'  <g fill="{hexed(INK)}">',
                *strokes,
                "  </g>",
                f'  <rect x="{middle - reach:g}" y="{middle - reach:g}" '
                f'width="{reach * 2:g}" height="{reach * 2:g}" fill="{hexed(SIGNAL)}"/>',
                "</svg>",
                "",
            ]
        ),
        encoding="utf-8",
        # Python in text mode writes `\r\n` on Windows. The repository stores LF, and
        # `.gitattributes` normalises on commit, so a CRLF file would check out as LF and
        # show up as modified the moment anything rewrote it.
        newline="\n",
    )


# The sizes a browser actually asks a favicon for. Not `SIZES`: a 256px frame in a
# favicon is sixty kilobytes of something nobody sees, and the browser picks from what
# is offered rather than scaling one of them itself.
FAVICON_SIZES = (16, 32, 48)

# The social card. 1200x630 is the crop every unfurler uses, and the mark is centred at
# a size that leaves the crop some ground on every side. There is no wordmark: the card
# is the same rule as the icon, which is one lamp on a dark tile and nothing else.
CARD = (1200, 630)
CARD_MARK = 360

TOUCH_ICON = 180


def write_docs(directory: Path) -> None:
    """Write the icon set the documentation site serves.

    Everything here comes from the same geometry as `zet.ico`, for the same reason
    `zet.svg` does: four copies of a mark are four marks, and only one of them gets
    edited. The site has no build step — GitHub Pages publishes `docs/` exactly as it is
    committed — so these are written into the repository and committed, not generated
    during deployment.
    """
    assets = directory / "assets"
    assets.mkdir(parents=True, exist_ok=True)

    favicon = directory / "favicon.ico"
    largest = draw_mark(FAVICON_SIZES[-1])
    frames = [draw_mark(size) for size in FAVICON_SIZES[:-1]]
    largest.save(
        favicon,
        format="ICO",
        sizes=[(size, size) for size in FAVICON_SIZES],
        append_images=frames,
    )
    print(f"wrote {favicon} ({favicon.stat().st_size} bytes, {len(FAVICON_SIZES)} sizes)")

    vector = assets / "icon.svg"
    write_svg(vector)
    print(f"wrote {vector} ({vector.stat().st_size} bytes)")

    # Square and opaque: iOS applies its own mask to a touch icon, and a transparent one
    # is composited onto whatever the home screen happens to be showing.
    touch = assets / f"icon-{TOUCH_ICON}.png"
    draw_mark(TOUCH_ICON, radius=0).save(touch, format="PNG")
    print(f"wrote {touch} ({touch.stat().st_size} bytes)")

    card = assets / "og.png"
    sheet = Image.new("RGBA", CARD, GROUND)
    mark = draw_mark(CARD_MARK)
    sheet.alpha_composite(mark, ((CARD[0] - mark.width) // 2, (CARD[1] - mark.height) // 2))
    sheet.convert("RGB").save(card, format="PNG")
    print(f"wrote {card} ({card.stat().st_size} bytes)")


def write_preview(path: Path) -> None:
    """Write a sheet of every size, on a mid grey so both edges of the tile show."""
    marks = [draw_mark(size) for size in SIZES]
    gap = 24
    width = sum(mark.width for mark in marks) + gap * (len(marks) + 1)
    height = max(mark.height for mark in marks) + gap * 2
    sheet = Image.new("RGBA", (width, height), (0x5A, 0x60, 0x68, 255))

    at = gap
    for mark in marks:
        sheet.alpha_composite(mark, (at, (height - mark.height) // 2))
        at += mark.width + gap

    sheet.save(path, format="PNG")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--preview",
        action="store_true",
        help=r"also write a sheet of every size to D:\Apps\tmp\zet\icon-preview.png",
    )
    parser.add_argument(
        "--docs",
        type=Path,
        metavar="DIR",
        help="also write the documentation site's icon set into DIR, which is `docs/`",
    )
    arguments = parser.parse_args()

    target = Path(__file__).with_name("zet.ico")
    write_ico(target)
    print(f"wrote {target} ({target.stat().st_size} bytes, {len(SIZES)} sizes)")

    vector = Path(__file__).with_name("zet.svg")
    write_svg(vector)
    print(f"wrote {vector} ({vector.stat().st_size} bytes)")

    if arguments.docs:
        write_docs(arguments.docs)

    if arguments.preview:
        TMP.mkdir(parents=True, exist_ok=True)
        sheet = TMP / "icon-preview.png"
        write_preview(sheet)
        print(f"wrote {sheet}")


if __name__ == "__main__":
    main()
