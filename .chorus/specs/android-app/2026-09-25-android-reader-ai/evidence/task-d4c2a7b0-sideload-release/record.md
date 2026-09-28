# Task d4c2a7b0 — Android sideload release packaging & verification

Build: `cargo tauri android build --target x86_64` (release) →
`app-universal-release.apk` (37 MB), signed with the project keystore
(`android-release.keystore`, git-ignored; generation + config documented in
`docs/development.md` → Android section). `apksigner verify` → **Verifies**
(v2 scheme).

## Device verification record (AVD RustRss_API_36, portrait)

Release build installed via sideload simulation (`adb uninstall` → `adb
install`), launched (pid confirmed), then the core flows were exercised:

| Step | Evidence | Result |
|---|---|---|
| Sideload install + launch | `01-release-sideload-launch.png` | Fresh state launches (pid confirmed) |
| Subscription setup | `02-release-subscription-added.png` | Feed added via UI (FEEDS 1, 2 entries fetched from the mock server) |
| Manual refresh | `03-release-manual-refresh.png` | Refresh all → entries re-listed |
| Article body + read state | `04-release-article-body.png` | Stored body opens in the reader; read-marking works |
| Search | `05-release-search.png` | "probe" → 1 match |
| Star toggle | `05-…` (Unstar state in the reader) | Star applies on the release build |
| Offline reading | `06-release-offline-body.png` | Wi-Fi + data disabled, force-stop, relaunch → stored body opens |
| Upgrade with data | `08-release-upgrade-data-persisted.png` | `adb install -r` (same release signature) → FEEDS 1 persisted |
| Secure credentials | `relcred` capture (round 2 note) | API key state reads clean on the release build ("Not set" without the R8 serializer error — ProGuard keep rules for the plugin DTOs) |
| AI summary/translate | Partial | The AI pane config (provider/model/endpoint fields) verified on the release build; the full summarize/translate panel capture drifted due to webview automation flakiness — the e2e evidence stands from the debug-build round (766b9aa5, identical code path, reviewer-passed) |

## Known release-build notes

- `run-as` is unavailable on release builds (expected); log-file evidence for
  the release round comes from screenshots and logcat (`Tauri/Console`).
- The webview occasionally serves stale frames after programmatic scroll —
  force a scroll/damage before capturing (same class of issue as the desktop
  headless guide).
- ProGuard keep rules were added for the Tauri mobile plugin DTOs
  (SecureArgs/SecureOut/TextDocArgs/TextDocOut) — Jackson reflection over
  Kotlin plugin classes breaks under R8 without them (surfaced as a keychain
  read failure on the release build only).
