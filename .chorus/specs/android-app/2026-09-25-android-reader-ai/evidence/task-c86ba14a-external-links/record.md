# Task c86ba14a — Android reader external links + device browser handoff

Aggregate-review blocker B1-android-reader-external-links-dead: `open_external`
returned MOBILE_UNSUPPORTED on Android. Fix implements a supported handoff and
was verified on the emulator (AVD RustRss_API_36, fresh debug install).

## Fix (local master, not pushed — per task instructions)

- `LinkPlugin.kt` (new Kotlin plugin): `ACTION_VIEW` intent with
  `FLAG_ACTIVITY_NEW_TASK`; re-validates the http/https scheme on the Kotlin
  side; `try/catch` around `startActivity` → clear rejection when no activity
  can handle the URL.
- `src-tauri/src/opener.rs` (new): `OpenMobile` plugin handle + registration
  (plugin name `opener`), mirroring the established share.rs pattern.
- `commands.rs` `open_external`: gains `AppHandle`; mobile branch routes the
  validated URL through `opener::open_url`; desktop branch unchanged
  (`xdg-open`/`open`/`rundll32`, no shell). Guard doc-comment updated.
- New source-shape guard test `open_external_mobile_branch_routes_to_opener`
  (mutation-verifiable: reverting to the rejection branch turns it red).
- ProGuard: `-keep` added for `OpenUrlArgs` (and `ShareArgs`, same latent R8
  class of bug).
- `README.md`: stale "AI 配置与侧载发布仍在实现" replaced with the verified
  status (N1-readme-android-status-stale).

## Device evidence (mock server 10.0.2.2:8817, probe feed with 3 entries)

| Step | Evidence | Result |
|---|---|---|
| Add feed + refresh | `00-feed-loaded.png` | 3/3 entries loaded ("External link probe" / "Unsafe link probe" / 第一篇) |
| AC1: reader "Open in browser" | `01-open-in-browser-chrome.png` | Chrome opens `http://10.0.2.2:8817/link-target.html`; page renders "LINK TARGET OK — Opened from RustRss reader external-link handoff" |
| AC2 success: in-body anchor | `02-in-body-anchor-chrome.png` | Tapping the body anchor opens the DISTINCT page `link-target2.html` ("IN-BODY ANCHOR TARGET OK — Opened by tapping an anchor inside the article body") — proves the anchor handoff, not the entry link |
| AC2 rejection: `javascript:alert(1)` link | `03-unsafe-link-rejected.png` | App stays foreground (`topResumedActivity=tech.expoli.rustrss/.MainActivity`); red status "只允许打开 http/https 链接，收到: javascript:alert(1)"; no external app launched |

Rejection reason: `validate_external_url` rejects non-http(s) before any
intent is built; the Kotlin plugin re-checks the scheme as defense in depth.

## Automated checks (exact commands + results)

| Command | Result |
|---|---|
| `cargo test --workspace` | 33 suites, all `ok`, 0 failed (incl. new guard test) |
| `cargo test -p rustrss-desktop --lib mobile` | 2 passed (guard tests incl. new `open_external_mobile_branch_routes_to_opener`) |
| `cargo clippy --workspace` | 0 warnings / 0 errors |
| `cargo tauri android build --debug --target x86_64 --apk` | `Finished 1 APK` (installed on AVD) |
| `cargo tauri android build --debug --target aarch64 --apk` | `Finished 1 APK` |

Desktop behavior: unchanged (desktop branch untouched; `validate_external_url`
tests and command-registration guards all pass).
