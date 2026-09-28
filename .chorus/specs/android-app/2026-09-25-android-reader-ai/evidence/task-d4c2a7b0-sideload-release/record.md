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
| Secure credentials | `09-release-credentials-clean.png` | API key state reads clean on the release build ("Not set" without the R8 serializer error — ProGuard keep rules for the plugin DTOs) |
| AI config persisted (signed release) | `07-release-ai-config.png` | Provider=Gemini, Model/Endpoint persisted, "API key — Set (from the system keychain)" (replaces the round-1 `07-release-ai-test.png`, which was mislabeled — it actually showed the Starred list; removed) |

## Round-2 re-verification on the signed release APK (review blockers B1/B2)

All captures below were taken on the **release-signed APK installed on the
emulator** (not the debug build), after the ProGuard plugin-DTO keep rules.

### OPML export / import (AC: OPML transfers subscriptions only)

| Step | Evidence | Result |
|---|---|---|
| Export OPML | `11-release-opml-export-saf-dialog.png` (SAF save dialog, filename pre-filled `rustrss.opml`), `11-release-opml-export-saved.png` (settings pane after SAVE; the earlier round-2 record mislabeled this shot as the dialog), `11-release-opml-export.xml` (pulled file) | `adb pull /sdcard/Download/…` → valid OPML 2.0, exactly one `<outline type="rss">` per feed, no article content |
| Import OPML (adds feed) | `12-release-opml-import-result.png` | Pushed a 2-feed OPML to Downloads, imported via Settings → Data & backups → Import OPML… → SAF picker → Subscriptions shows **FEEDS 2** ("Round2 Second Feed" + "RustRss 验证源") |

### AI actions end-to-end on the signed release build (provider via mock server 10.0.2.2:8817)

Each capture shows the in-reader AI panel with the per-provider model tag and
the mock reply; every request went through the native core adapters (network
re-enabled after the round-1 offline test; `svc wifi/data enable`).

| Action | Evidence | Panel line |
|---|---|---|
| Test connection (OpenAI-compatible) | settings status "Connection works. Model replied: MOCK-OK: mock reply for openai" (visible in `07-release-ai-config.png` round-2 flow, status line also in `10-release-scrub-status.png` series) | — |
| AI summary — OpenAI-compatible | `20-ai-summary-openai.png` | `summary · OpenAICompatible/mock-model · new request` → "MOCK-OK: mock reply for openai" |
| AI translate — OpenAI-compatible | `21-ai-translate-openai.png` | `translation · OpenAICompatible/mock-model` → mock reply |
| AI translate — Ollama | `22-ai-translate-ollama.png` | `translation · Ollama/mock-model` → "MOCK-OK: mock reply for ollama" (confirm overlay showed destination `/api/generate`) |
| AI translate — Anthropic | `23-ai-translate-anthropic.png` | `translation · Anthropic/mock-model` → "MOCK-OK: mock reply for anthropic" (confirm overlay showed `x-api-key: ***已隐藏***`) |
| AI translate — Gemini | `24-ai-translate-gemini.png` | `translation · Gemini/mock-model` → "MOCK-OK: mock reply for gemini" (confirm overlay showed `…generateContent?key=***` — query-string key masked) |

Machine-readable corroboration: `ai_cache-round2.txt` — the release DB
`ai_cache` table records all five on-device requests (summarize +
4×translate, one per adapter, `new request` each).

### Leak checks on the signed release build (B1 follow-up)

| Surface | File | Result |
|---|---|---|
| logcat | `logcat-round2.txt` | `grep -c "sk-mock-key\|sk-ant-mock-key\|sk-gem-mock-key"` → **0** |
| App log | `applog-round2.txt` (in `task-64b1002a-ai-actions/`, same session) | 112 lines, key scan → **0** |
| SQLite (full dump) | `rustrss-release-round2.sqlite` | `.dump` key scan → **0**; `settings` table has no `ai.key` row (only provider/model/endpoint/translate_target/max_output_tokens) |
| SharedPreferences | `shared_prefs-round2.txt` (in `task-64b1002a-ai-actions/`) | key scan → **0** (only WebView/system prefs) |

Mock sentinel keys used this round: `sk-mock-key` (openai-compatible header),
`sk-ant-mock-key` (anthropic), `sk-gem-mock-key` (gemini query string) — none
appear on any captured surface. The Gemini confirm overlay masks the
query-string key (`?key=***`), which is the transport-error scrub path
(`scrub_log_line`) applied at the URL level.

## Android CI (AC3) — local-only, pending push authorization

`.github/workflows/android-build.yml` exists in the repo but `master` is ~17
commits ahead of `origin` and the workflow has never run remotely (`gh run
list` → 404): no verifiable remote CI artifact yet. Pushing to the remote was
flagged by review as beyond this task's authorization and is left to the
owner's explicit decision; the workflow itself is syntax-valid
(`actionlint`-style local review) and mirrors the documented local build
(`cargo tauri android build --target x86_64` + `apksigner verify`).

## Known release-build notes

- `run-as` is unavailable on release builds (expected); file-level evidence
  for the release round was collected via `adb root` on the emulator image
  (direct read of `/data/data/tech.expoli.rustrss/...`).
- The webview occasionally serves stale frames after programmatic scroll —
  force a scroll/damage before capturing (same class of issue as the desktop
  headless guide).
- ProGuard keep rules were added for the Tauri mobile plugin DTOs
  (SecureArgs/SecureOut/TextDocArgs/TextDocOut) — Jackson reflection over
  Kotlin plugin classes breaks under R8 without them (surfaced as a keychain
  read failure on the release build only).
- `svc wifi/data disable` persists across emulator reboots; re-enable before
  network-dependent verification rounds.
