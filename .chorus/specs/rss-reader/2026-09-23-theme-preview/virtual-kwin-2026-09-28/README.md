# Theme preview in an isolated KWin Wayland compositor (2026-09-28)

This is runtime evidence for Chorus task `ee4cce71-d7a1-46ed-a98b-366e2de78ec5`. The tested source was RustRss `b7bfada`; exact hashes and UTC build times of the fixture, desktop, and MCP binaries are in [results.json](results.json). The source and binaries predate the Windows/macOS adapter commits. All databases and captured content were fixed fixtures; the user's RSS database was not opened.

After locally integrating the Windows and macOS adapters, source `835d551` was rebuilt and both virtual compositor sessions were repeated. [integrated-results.json](integrated-results.json) records the new desktop, MCP, and fixture binary SHA-256 values and mtimes, 39 fixture/24 product capture manifests per session, distinct three-scene hashes in every cell, the product's revision/configuration hash and capture geometry, and the six archived `integrated-*.png` originals. Both sessions passed the same assertions. This is the acceptance evidence for the integrated code; the original manifest above remains as the before-integration baseline.

## What ran

- `kwin_wayland --virtual --no-lockscreen` on a private Wayland socket and private D-Bus session, once at output scale 1 and once after `kscreen-doctor output.Virtual-0.scale.1.5`.
- Rebuilt `theme_ui` with `snapshot-probe`; each virtual session produced 39 native WebView PNGs and passed 47 fixture checks. In each of 12 preset/mode/locale cells, the overview, article, and settings PNGs had **three distinct SHA-256 hashes**. The headless Xvfb result in [empty-state-matrix.md](../empty-state-matrix.md) had only one frame per cell; that limitation does not apply to these virtual KWin sessions.
- Rebuilt `rustrss-desktop`, `rustrss-mcp`, and `theme_fixture`; `verify-theme-preview.py --display wayland` produced 24 production MCP preview PNGs in each session. In all six preset/mode cells, the three scene files had distinct hashes. The probe asserted its native freshness marker, PNG dimensions, file size ceiling, temporary zero write, stdio bridge, save revision, idempotent save, and token revocation behavior. `results.json` records every capture's revision, config hash, size, backend, and SHA-256.
- The six archived raw PNGs are the `clear/light` overview, article, and settings trio at each output scale. I opened and visually inspected these actual image files: the overview has an empty reading pane, the article has the fixed heading/body/code/diff, and settings has the appearance dialog covering the list. The 1.5 output setting was confirmed by `kscreen-doctor`; `wl_output` and WebKit used integer buffer scale **2**, so the PNG was 2× the logical WebView content dimensions. This verifies the app's reported content-to-PNG geometry in that compositor, not subjective sharpness on a physical 150% monitor.

## Reproduction

Build from the intended RustRss checkout before running; UI assets are embedded at build time:

```sh
cargo build --workspace --features rustrss-desktop/snapshot-probe --bins --example theme_ui --example theme_fixture
```

In a private `dbus-run-session`, create a mode-0700 `XDG_RUNTIME_DIR`, unset the parent `DISPLAY` and `WAYLAND_DISPLAY`, start `kwin_wayland --virtual --socket wayland-0 --width 1600 --height 1000 --no-lockscreen --no-global-shortcuts`, and wait for its socket. Set `WAYLAND_DISPLAY=wayland-0`, then run `/usr/bin/python3 scripts/verify-theme-ui.py --display wayland` and `/usr/bin/python3 scripts/verify-theme-preview.py --display wayland`. For the second run, call `kscreen-doctor output.Virtual-0.scale.1.5` before either probe. Keep the printed temporary output directories if regenerating a capture manifest; stop only the KWin process started for this run. The probes create isolated fixture databases and exit without touching the user's application instance.

## Boundaries and open checks

| Check | Result |
| --- | --- |
| Virtual KWin Wayland, output scale 1, three scene pixels and MCP metadata | PASS |
| Virtual KWin Wayland, output scale 1.5, WebView buffer scale 2 and three scene pixels | PASS, virtual output only |
| Physical KDE Wayland session in this daemon run | OPEN: session was locked; a direct fixture attempt reported `GdkWaylandDisplay` but produced no PNG before the 100-second harness timeout. It is not counted as a product failure or pass. |
| GNOME Wayland | OPEN: no GNOME compositor installed on this host. |
| Real minimize/restore | OPEN: not exercised by the virtual harness; a virtual output cannot substitute for physical-window behavior. |
| Cross-monitor movement and human sharpness assessment | OPEN: virtual KWin had one output. Earlier physical KDE observations remain separately documented in [native-settings.md](../native-settings.md). |

No product code was changed to obtain these results. The stored six PNGs cover two claimed virtual sessions; the larger fixture and MCP matrices have SHA-256 manifests in `results.json`, with full temporary outputs on the test host only.
