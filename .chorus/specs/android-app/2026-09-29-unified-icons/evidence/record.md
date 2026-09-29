# Unified application icons (2026-09-29)

Chorus task: `92539caf-b917-4c12-b475-9a7cb2a1ece5`.

## Scope and cause

The desktop PNG used Ferris + blue RSS, but Android native `res/` still held the
stock Tauri yellow/cyan motif. Separate `icons/android/` and `icons/ios/` copies
also used older RSS artwork. Android packaging consumed its native resources,
so updating desktop bundle icons alone did not update the APK.

`scripts/sync-icons.py` generates all checked-in platform derivatives from
`src-tauri/icons/src/rustrss-icon.svg` using Tauri CLI 2.12.0, then explicitly
synchronizes Android native build inputs. It records source/script and output
hashes in `provenance.json`. Desktop PNG/ICO/ICNS and Windows store resources,
Android five densities and iOS artwork all share the same SVG and palette.
AndroidManifest references normal and round variants; API26+ references use the
same adaptive foreground and background. Unused stock Tauri drawable vectors
were removed to avoid accidental future references.

Adaptive foreground removes the background and scales native SVG geometry to
60%; the solid background is `#1c1f26`. Read-only pixel inspection measures the
nontransparent motif at radius 32.045dp, within the 33dp safety circle on the
108dp canvas. [Official Android icon guidance](https://developer.android.com/codelabs/basic-android-kotlin-compose-training-change-app-icon)
explains this safe area. The launcher still chooses its outer mask.

## Checks

- `before-check.log`: regression-first exit 1 on stale Android copies, missing
  adaptive/round configuration and provenance. No repair had run at this point.
- `consistency-check.log`: post-repair pass; deliberate mutation of one native
  PNG and of the canonical SVG each produces exit 1, then restored resources pass.
- Both regular CI and Android build CI run `python3 scripts/sync-icons.py --check`
  without needing the icon-generation CLI.
- `apk-icon-pixels.json`: APK legacy, round and adaptive foreground PNG pixels
  match the generated xxxhdpi resources, despite Android's PNG recompression and
  shortened resource names.
- Workspace tests and strict Clippy passed; retained logs document results.

## Signed preview and boundaries

Preview APK: `RustRss_0.2.0_android-unified-icons-preview.apk`,
SHA256 `605c1815bf544174d1385048f287b5f9701f1aa99a974c29d6ed8351c3802955`.
Original certificate SHA256
`89269c116afaa9ca546f7287b268f00c14d5598b6c633715a74fbbcd28de8cc5`;
ARM64/x86_64, version 0.2.0/code2000, release/non-debuggable.

Owned API36 x86_64 emulator-5580 / Pixel Launcher: before screenshot shows the
Tauri icon in All apps; after signed install-r screenshot shows the Ferris + RSS
adaptive icon at the same entry. Launching from that new entry opens RustRss.
Formal v0.2.0 tag and release assets remain unchanged.

Windows/macOS/iOS native launcher display and other Android launcher masks are
not claimed as device-tested. Their checked-in derivatives are generated and
hash-checked; iOS resources do not imply an iOS implementation. Shared user
emulator-5554 is untouched; all install/uninstall steps operate only on the
owned fixture device, not user data.

Signed smoke `results.json` confirms unchanged UID, retained prior XML feed,
new generic OPML import, retained settings category home and launch from the
new launcher entry. `signed-smoke.py` retains the native UI procedure and exact
preview paths for this run; fixture files were already present in the owned
Downloads directory from the prior OPML regression. `signed-smoke.log` records
actual install and install-r Success. Reproduction on a fresh fixture device
requires first pushing the XML/generic OPML fixtures from the OPML regression
script; never run its uninstall step on a user device.

## Independent verification and completion

Round1 task review `623cec11-8251-471a-931d-a77e0453ad44` posted VERDICT PASS,
4AC passed, zero blockers/notes; original comment retained in
`independent-review.md`. Fresh read-only resource/mutation checks, APK checks,
workspace tests/strict Clippy/build and live Pixel Launcher navigation passed.
Independent screenshots include all derivative/ICO/ICNS representations,
live launcher and native OPML picker in `independent/`; reviewer did not claim
to rerun developer's chronological signed upgrade. Chorus task92539caf is done.

Production source commit `1d08d9eb113d5d3e96049e98af0e0110187e5eff` has successful
[CI](https://github.com/expoli/RustRss/actions/runs/36531797786) and
[Android build](https://github.com/expoli/RustRss/actions/runs/36531797893).
Owned emulator-5580 was closed after review release; shared emulator-5554 remains
untouched. Final retention changes add only evidence/docs, not product code.
