# RustRss app icon — design notes

**File:** `rustrss-icon.svg` — 512×512, `viewBox="0 0 512 512"`, flat: no gradients, no strokes, no text, no shadows.

**Palette:** surface `#1c1f26` (app dark theme) · Ferris `#F74C00` (shell/arms/pincers) + `#E03E00` (legs, one step darker for depth) · RSS signal `#4c9aff` (app accent) · eye white `#e6e8ec` · pupils/smile `#1c1f26` (= background ink). Orange is the only warm colour, so Ferris is the single focal mass.

**Geometry (px @512):**
- Background: full-bleed rounded rect 512×512, corner r=104 (~20 %, superellipse-ish), transparent outside the radius.
- Ferris: shell ellipse 200×156 centred (302,256); two r=28 eyes on the shell's top edge (262/342,168) with r=13 pupils + a r=29/w=12 smile; 2 legs per side (w=22 capsules, darker orange, tucked behind the shell); w=26 arm stubs; pincers = 230° ring segments (centreline r=30, w=42, round caps) whose 130° mouths open up/outward — shell + eyes + two claws stay legible when everything else collapses.
- RSS: solid dot r=27 at (92,412) + two concentric quarter arcs (r=60/w=30 and r=116/w=34, round caps, 270°→360°), i.e. the classic feed glyph, tucked into the bottom-left corner.
- Artwork bbox 65–467 × 139–439 → margins 12.7 % left, 8.8 % right, 14.3 % bottom, 27 % top (crab is canvas-centred per brief, so the slack sits above the claws). No element is clipped by the rounded rect.

**Rationale:** orange Ferris vs. cool blue signal on a dark surface gives maximum hue+luma separation, and the two never overlap (no low-contrast seams). Squares, circles and thick capsules only, so at 32 px (KDE taskbar) the icon still reads: orange body + two light eyes, claw bumps at the sides, blue dot + two arcs bottom-left. Every element is a closed **filled** subpath (round caps/joins are baked into the path data), which makes ImageMagick's internal SVG renderer — it silently drops all strokes — produce the same picture as WebKitGTK/Chrome/librsvg.

**Verification:** `convert -background none rustrss-icon.svg -resize 512x512 check-512.png` and `… -resize 32x32 check-32.png`, both inspected; `check-32-zoom.png` is the 32 px render at 12× nearest-neighbour zoom. `generate_icon.py` regenerates the SVG from the numbers above (edit it, not the path data, when tweaking).
