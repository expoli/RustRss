#!/usr/bin/env python3
"""RustRss app icon generator.

Emits /tmp/icon-design/rustrss-icon.svg using ONLY filled geometry
(no strokes): every ring, capsule and arc is a closed filled subpath.
That keeps the icon identical in strong renderers (WebKitGTK/Chrome/rsvg)
AND in weak ones (ImageMagick's internal MSVG renderer drops strokes).

Screen coordinates: x right, y down, angles in degrees (0 = +x, 90 = +y/down).
"""
import math

W = H = 512


def P(ra, a, cx, cy):
    t = math.radians(a)
    return (cx + ra * math.cos(t), cy + ra * math.sin(t))


def f(x):
    return f"{x:.2f}".rstrip("0").rstrip(".")


def ring_segment(cx, cy, r_c, width, a0, a1):
    """Annulus segment from a0 to a1 (a1 > a0, clockwise), with round caps."""
    ro, ri = r_c + width / 2, r_c - width / 2
    hw = width / 2
    large = 1 if (a1 - a0) > 180 else 0
    pts = [
        P(ro, a0, cx, cy), P(ro, a1, cx, cy),
        P(ri, a1, cx, cy), P(ri, a0, cx, cy),
    ]
    (x0, y0), (x1, y1), (x2, y2), (x3, y3) = pts
    # outer arc travels clockwise (increasing angle); for that winding both
    # round caps are drawn clockwise in their own local frames (sweep=1)
    return (f"M {f(x0)} {f(y0)} "
            f"A {f(ro)} {f(ro)} 0 {large} 1 {f(x1)} {f(y1)} "
            f"A {f(hw)} {f(hw)} 0 0 1 {f(x2)} {f(y2)} "
            f"A {f(ri)} {f(ri)} 0 {large} 0 {f(x3)} {f(y3)} "
            f"A {f(hw)} {f(hw)} 0 0 1 {f(x0)} {f(y0)} Z")


def capsule(x0, y0, x1, y1, width):
    """Stadium/capsule with round ends from (x0,y0) to (x1,y1)."""
    dx, dy = x1 - x0, y1 - y0
    L = math.hypot(dx, dy)
    ux, uy = dx / L, dy / L
    px, py = -uy * width / 2, ux * width / 2
    hw = width / 2
    ax, ay = x0 + px, y0 + py
    bx, by = x1 + px, y1 + py
    cx_, cy_ = x1 - px, y1 - py
    dx_, dy_ = x0 - px, y0 - py
    # p = (-uy, ux)*hw puts A+p/B+p on one side; for that winding both end
    # caps sweep negative (counter-clockwise in their local frames)
    return (f"M {f(ax)} {f(ay)} L {f(bx)} {f(by)} "
            f"A {f(hw)} {f(hw)} 0 0 0 {f(cx_)} {f(cy_)} "
            f"L {f(dx_)} {f(dy_)} "
            f"A {f(hw)} {f(hw)} 0 0 0 {f(ax)} {f(ay)} Z")


def circle(cx, cy, r, fill):
    return f'<circle cx="{f(cx)}" cy="{f(cy)}" r="{f(r)}" fill="{fill}"/>'


def path(d, fill):
    return f'<path d="{d}" fill="{fill}"/>'


# ---------------------------------------------------------------- palette
BG = "#1c1f26"      # app dark theme surface
ORANGE = "#F74C00"  # Ferris shell / claws / arms
ORANGE_D = "#E03E00"  # Ferris legs (darker tone = depth)
BLUE = "#4c9aff"    # app accent = RSS signal
INK = "#1c1f26"     # pupils / smile (same as background)
EYE = "#e6e8ec"     # eye white

out = []
out.append(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" '
           f'width="512" height="512">')
out.append("  <title>RustRss</title>")
out.append("  <!-- Filled-only geometry (no strokes) so every renderer, "
           "incl. ImageMagick MSVG, draws it identically. -->")

# ------------------------------------------------- background (full bleed)
out.append(f'  <rect width="512" height="512" rx="104" fill="{BG}"/>')

# ------------------------------------------------------- RSS signal bottom-left
# dot + 2 concentric quarter arcs (270deg -> 360deg), round caps
# 点半径 27→21：点与内弧的间隙 18→~28 design u，32px 下不再粘连
# （终审实测：27 时间隙仅 1.1px，1/3 弧段蓝覆盖 0.7 = 点弧融合）
out.append("  <!-- RSS: solid dot + 2 concentric quarter arcs (bottom-left) -->")
out.append("  " + path(ring_segment(92, 412, 118, 34, 270, 360), BLUE))
out.append("  " + path(ring_segment(92, 412, 60, 30, 270, 360), BLUE))
out.append("  " + circle(92, 412, 21, BLUE))

# --------------------------------------------------------------- Ferris legs
out.append("  <!-- Ferris: 2 legs per side (behind the shell) -->")
legs = [(244, 300, 226, 332), (276, 318, 266, 348),
        (360, 300, 378, 332), (328, 318, 338, 348)]
for x0, y0, x1, y1 in legs:
    out.append("  " + path(capsule(x0, y0, x1, y1, 22), ORANGE_D))

# --------------------------------------------------------------- Ferris shell
out.append("  <!-- Ferris: rounded oval shell -->")
out.append(f'  <ellipse cx="302" cy="256" rx="100" ry="78" fill="{ORANGE}"/>')

# ---------------------------------------------------------------- Ferris arms
out.append("  <!-- Ferris: arms -->")
out.append("  " + path(capsule(252, 214, 194, 210, 26), ORANGE))
out.append("  " + path(capsule(352, 216, 394, 210, 26), ORANGE))

# --------------------------------------------------------------- Ferris claws
# chunky open pincers (230deg of ring), mouths facing up/outward
out.append("  <!-- Ferris: chunky open pincers -->")
out.append("  " + path(ring_segment(168, 190, 30, 42, 280, 510), ORANGE))
out.append("  " + path(ring_segment(416, 190, 30, 42, 30, 260), ORANGE))

# ----------------------------------------------------------------- Ferris face
out.append("  <!-- Ferris: eyes + smile -->")
out.append("  " + circle(262, 168, 28, EYE))
out.append("  " + circle(342, 168, 28, EYE))
out.append("  " + circle(264, 172, 13, INK))
out.append("  " + circle(340, 172, 13, INK))
out.append("  " + path(ring_segment(302, 193, 29, 12, 46.4, 133.6), INK))

out.append("</svg>")
svg = "\n".join(out) + "\n"

with open("/tmp/icon-design/rustrss-icon.svg", "w") as fh:
    fh.write(svg)
print(svg)
