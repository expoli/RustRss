# Task 766b9aa5 — Android subscription, refresh, and offline reading flows

Device: AVD `RustRss_API_36` (x86_64, API 36, portrait 1080×2400 → CSS 412×839).
Build: `cargo tauri android build --target x86_64 --debug`.
Host test server: `python3 -m http.server 8817` serving `feed.xml` (RSS) and
`site.html` (HTML with `<link rel="alternate" type="application/rss+xml">`),
reachable from the emulator at `http://10.0.2.2:8817`.

## What was implemented (this task)

1. **Foreground-resume refresh** (`scheduler.rs::resume_refresh` + `lib.rs`):
   `tauri-runtime-wry` delivers Android resume as **`WindowEvent::Resumed` to
   per-window listeners** (not `RunEvent::Resumed`, which is desktop-only
   polling legacy) — the listener is registered on the main window in setup and
   triggers the same single-flight full refresh used by manual/startup paths.
   A `try_state` guard skips the initial resume that can arrive before app state
   exists (startup refresh covers it).
2. **OPML via Android document selection** (`documents.rs` + `DocumentsPlugin.kt`):
   `blocking_pick_file`/`blocking_save_file` return `content://` URIs on Android
   which `std::fs` cannot read/write — the new plugin bridges them through
   `ContentResolver` streams; `import_opml`/`export_opml` branch on platform.
3. Everything else (add/discover/edit/remove, OPML core logic, search, read/
   star/read-later, offline lists/bodies) already existed in the shared core/commands
   and is exercised as-is by the phone UI.

## Evidence

| File | AC | Shows |
|---|---|---|
| `01-discovered-from-website.png` | AC-1 | Website URL added → log `discover_feed …/site.html -> …/feed.xml via=LinkType` → feed added |
| `02-feed-renamed.png` | AC-1 | Long-press → Edit feed dialog (custom title "RustRss", read-only URL) → sidebar shows renamed feed; status bar "Saved: RustRss" |
| `02b-unsubscribe-confirm.png` / `02c-feed-removed.png` | AC-1 | Long-press → Unsubscribe → confirm dialog ("removes 'RustRss' and all its articles") → FEEDS 0 |
| `03-manual-refresh-entries.png` | AC-2 | Manual Refresh all → 3 entries fetched and listed |
| `04-article-body.png` | AC-2/3 | Full-screen reader with stored body + touch actions |
| `05-offline-list.png` | AC-3 | Wi-Fi + data disabled, force-stop, relaunch → list loads from SQLite ("loaded 2 / 2 unread") |
| `06-offline-article-body.png` | AC-3 | Stored body opens offline |
| `08-offline-search-probe.png` | AC-3 | Offline search "probe" → 1 match |
| `09-offline-starred.png` | AC-3 | Star toggled offline (Unstar state) |
| `10-restart-starred-persisted.png` | AC-3 | After force-stop + relaunch (still offline): Saved → Starred shows the probe entry (1/1) |
| `11-export-save-sheet.png` / `11-export-saved-sheet.png` | AC-1/4 | Export OPML opens the system CREATE_DOCUMENT sheet in Downloads → saved as `rustrss.opml.xml` |
| exported OPML (content below) | AC-4 | File contains **only** the subscription outline — no article cache, read state, or AI settings |
| `12-import-picker.png` / `13-import-result.png` / `14-import-saved-empty.png` | AC-1/4 | `pm clear` (fresh state) → Import OPML via GET_CONTENT → log `import_opml added=1 skipped=0`; "Nothing unread" + Starred empty → subscriptions only |

Exported OPML (pulled from `/sdcard/Download/rustrss.opml.xml`):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <head>
    <title>RustRss 订阅</title>
  </head>
  <body>
    <outline type="rss" text="RustRss 验证源" title="RustRss 验证源" xmlUrl="http://10.0.2.2:8817/feed.xml" htmlUrl="http://example.invalid/"/>
  </body>
</opml>
```

## Refresh timing proof (AC-2, app log)

```
09:57:08 scheduler: 自动刷新完成: fetched=0 not_modified=1 …   ← startup refresh (launch + 10 s)
09:57:17 scheduler: 前台恢复：触发一次刷新                      ← resume (HOME → am start)
09:57:17 scheduler: 自动刷新完成: fetched=0 not_modified=1 …   ← resume refresh done
09:43:32 ui: add_feed id=1 …                                    ← add after discovery
09:58:03 scheduler: 自动刷新完成: … failures=1                  ← offline refresh attempt (network disabled) — app stayed up
```

## Repro steps

```bash
# discovery + add (website URL)
adb shell input tap 408 2250        # Subscriptions
adb shell input tap 938 207         # Add feed
adb shell input tap 450 1990        # URL input
adb shell input text 'http://10.0.2.2:8817/site.html'
adb shell input keyevent 4          # close keyboard
adb shell input tap 975 1990        # Add → discover → add → first fetch
# edit: long-press feed row → Edit → title → Save
adb shell input swipe 300 450 300 450 800
adb shell input tap 365 618         # Edit
# remove: long-press → Unsubscribe → Delete
# OPML export: Settings → Data & backups → Export OPML… → SAVE (Downloads)
# OPML import: pm clear → Settings → Data & backups → Import OPML… → pick file
# offline: adb shell svc wifi disable && adb shell svc data disable
```

## Round 2 — read-later persistence capture (review note N1)

Dedicated before/after device capture for read-later persistence:

1. `16-readlater-flag-on-articles.png` — Articles view with the probe entry's
   ⚑ read-later flag enabled (tap on the row flag).
2. `15-readlater-persisted.png` — after `am force-stop` + relaunch:
   Saved → **Read later** segment lists the entry ("loaded 1 / 1 total").

The starred-state capture (`10-restart-starred-persisted.png`) remains from
round 1; read-later now has its own equivalent artifact.
