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
- [x] Add and manage RSS subscriptions by URL, discover feeds from site URLs,
  and import/export OPML.
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

## Non-goals
- iOS support.
- Cloud accounts or cross-device sync of subscriptions, articles, reading
  state, or AI settings.
- Guaranteed background refresh or background-refresh notifications.
- Google Play publication in the first Android release.
- Running the desktop MCP server, system tray, or desktop single-instance
  behavior on Android.
