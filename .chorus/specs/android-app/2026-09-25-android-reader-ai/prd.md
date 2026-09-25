---
title: PRD: RustRss Android Reader and AI
proposalUuid: 5bd2ee27-6a49-492d-9923-aae49eadabe6
documentUuid: ade5e0ed-4829-439e-8458-477617b7e6d3
---

# PRD: RustRss Android Reader and AI

## Background

RustRss currently ships as a Tauri 2 desktop application for Windows, macOS,
and Linux. Its Rust core already owns feed parsing, fetching, SQLite storage,
article processing, and AI provider adapters. The requested Android app should
reuse that work while providing a phone-first reader.

Each Android installation is an independent local-first instance. Users can
bring subscriptions in through URL entry or OPML; this effort does not add an
account or synchronize data with desktop installations.

## Requirements

### Functional Requirements

- **FR-1 — Android install and launch:** produce a release APK that can be
  installed by sideloading and starts into the RustRss reader.
- **FR-2 — Subscription setup:** add a feed URL or website URL, discover a feed
  from a site URL, edit/remove subscriptions, and import/export OPML.
- **FR-3 — Refresh:** refresh manually, at startup, and when returning to the
  foreground. Background periodic refresh is not promised in this release.
- **FR-4 — Article reading:** browse the feed/article list, open cached article
  content, search, and update read/unread, starred, and read-later state.
- **FR-5 — Offline reading:** previously stored article lists and content remain
  readable without a network connection. A full-text fetch that has not yet
  been stored cannot be read offline.
- **FR-6 — AI setup and use:** configure OpenAI-compatible APIs, Anthropic,
  Gemini, and Ollama, including model and custom endpoint settings where
  supported by the existing adapter; test connectivity; summarize and
  translate an article.
- **FR-7 — Credential protection:** keep provider API keys in Android secure
  storage; never persist them as plaintext in SQLite, app preferences, or logs.
- **FR-8 — Phone-first UI:** provide a single-column, touch-friendly reader for
  portrait phones. Tablets should remain usable, but are not a first-release
  acceptance target.
- **FR-9 — Local data:** store the SQLite database inside the Android
  application sandbox. OPML transfers subscriptions only; it does not transfer
  articles, reading state, or AI configuration.

### Non-Functional Requirements

- Reuse `rustrss-core` for feed parsing, storage, refresh, search, and AI
  provider behavior where those interfaces are platform-neutral.
- Keep Android-specific storage and lifecycle behavior outside the core crate.
- Preserve existing desktop build and runtime behavior while gating
  desktop-only facilities from Android.
- Keep the existing static JavaScript frontend without introducing a frontend
  build framework solely for Android.
- Sideloaded APK installation and a documented upgrade path are the initial
  distribution target.

## User Stories

- As an Android reader, I want to import my subscriptions or add a feed URL so
  I can begin reading without syncing a desktop database.
- As an Android reader, I want saved articles available offline and their
  reading states kept on this device.
- As an AI user, I want to configure my own provider credentials and summarize
  or translate an article from the reader.

## Out of Scope

- iOS builds or iOS-specific UX.
- Cloud accounts and cross-device synchronization.
- Guaranteed periodic/background refresh and associated background notices.
- Google Play publication, in-app purchases, or subscription billing.
- Android-hosted MCP service, desktop system tray, and desktop single-instance
  behavior.
- Bundling local Ollama models or an Ollama server. An Ollama endpoint must be
  reachable from the Android device.
- New AI capabilities beyond provider configuration, connectivity testing,
  article summaries, and article translations.

## Acceptance Summary

- Install the release APK on an Android phone and complete add/import, refresh,
  list/search, open/read, state update, and offline-reading flows.
- Verify the database resides in app-private storage and survives app restart.
- Configure each existing provider type with test credentials or a mock
  endpoint, verify connection testing, and run summary and translation flows.
- Verify API keys survive restart through secure storage and do not appear in
  the database or logs.
- Verify refresh runs on startup/resume and user request, without asserting
  work while the app is suspended.
- Verify desktop CI/build remains functional.

## References

- Tauri mobile project structure: https://v2.tauri.app/start/project-structure/
- Tauri Android prerequisites: https://v2.tauri.app/start/prerequisites/
- Android secure keyring options: https://docs.rs/crate/keyring/latest/features
