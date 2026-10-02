---
slug: android-app
title: RustRss Android App
status: active
created: 2026-09-25
---

## Intent
Provide an Android-first RustRss reader that reuses the existing Rust core and
supports local reading and the existing BYOK article summary and translation
features. Each device keeps its own local database; no cloud account or
cross-device synchronization is part of this capability.

## Requirements
- [x] Install and run a sideloadable Android APK.
- [x] Share the desktop Ferris + RSS application artwork and dark palette;
  synchronize regular/round/adaptive launcher resources with native build
  inputs. API36 Pixel Launcher screenshot verified; other device launchers
  remain untested. [Icon verification](2026-09-29-unified-icons/evidence/record.md).
- [x] Add and manage RSS subscriptions by URL, discover feeds from site URLs,
  and import/export OPML.
- [x] Allow Android OPML import from documents identified as generic MIME types;
  validate OPML content after selection, retain duplicate/error/cancel semantics
  and desktop OPML/XML filtering. API36 Downloads evidence:
  [OPML picker verification](2026-09-29-opml-picker/evidence/record.md).
- [x] Refresh on user request, startup, and return to the foreground; do not
  promise periodic background refresh.
- [x] Browse, search, and read cached articles offline; update read, starred,
  and read-later states.
- [x] Configure OpenAI-compatible, Anthropic, Gemini, and Ollama providers;
  use configured providers for article summaries and translations.
- [x] Keep the database in Android app storage and API keys in secure storage.
- [x] Prioritize portrait phone usability; tablets remain usable but are not a
  first-release acceptance target.
- [x] Use page-based phone navigation instead of a compressed three-pane
  layout: Articles is the default home, Subscriptions and Saved are peer
  destinations, Settings is a peer destination, and opening an article pushes
  a full-screen reader that returns to the preserved list state.
- [x] Keep subscription, reading and AI inputs visible above the Android IME;
  use a top subscription entry and scroll the focused control into the resized
  viewport (API 36 x86_64 / Gboard device evidence, 2026-09-29).
- [x] Use a vertical settings category list with full-width details and Android
  Back from detail to categories before closing. Stack phone forms, collapse
  advanced typography/AI options, and preserve explicit save/discard behavior;
  verify 360/412 CSS px widths and desktop layout. Evidence:
  [phone UX verification](2026-09-29-phone-ux/evidence/record.md).
- [x] Keep the complete primary bottom navigation on Articles, Subscriptions,
  Saved and Settings, including category details; size the shell from actual
  content instead of a fixed footer estimate, and prevent toolbar overflow from
  enlarging the Android layout viewport. API36 x86_64, gesture/three-button
  system modes and 14/18/24 CSS px UI fonts verified; physical ARM64 and Android
  system font scaling remain untested. [Navigation evidence](2026-09-29-primary-navigation/evidence/record.md).
- [x] Enable native Android WebView history Back explicitly; verify Settings
  detail to categories to source destination and reader return from middle/end
  of a populated list without replacing its rows or losing scroll position.
  Unsaved theme drafts are discarded when leaving Settings; desktop keeps its
  modal/focus behavior. See the navigation evidence above for runtime limits.
- [ ] Expose on-device diagnostic logs from Settings → Data: list all retained
  `rustrss-*.log` files (name, size, mtime, current-startup marker), view the
  tail (256KB default) with an explicit truncation notice, and export the full
  file through the system document picker (CREATE_DOCUMENT), reusing the OPML
  export pipeline. File access is name-whitelisted (`rustrss-*.log`) against
  path traversal; export keeps the on-disk scrub discipline (no re-scrub) and
  refuses files over 16MB with a readable error. Desktop gets no entry this
  round. [Change folder](2026-10-01-log-viewer-export/)
 勾选注（未勾）：代码、双平台编译、全量测试与 clippy 已验证（af3c5f2..4e30757，
  三任务独立评审 + 聚合代码复审均 PASS）；**实机（Android 设备/模拟器）上的块可见性、
  查看截断提示与 CREATE_DOCUMENT 导出尚无运行时证据**，待人工核验后凭据勾选。

## Non-goals
- iOS support.
- Cloud accounts or cross-device sync of subscriptions, articles, reading
  state, or AI settings.
- Guaranteed background refresh or background-refresh notifications.
- Google Play publication in the first Android release.
- Running the desktop MCP server, system tray, or desktop single-instance
  behavior on Android.
