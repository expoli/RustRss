---
title: Tech Design: RustRss Android Reader and AI
proposalUuid: 5bd2ee27-6a49-492d-9923-aae49eadabe6
documentUuid: 32b6ed0f-6cad-4a3f-9822-8d4a6ecbcd61
---

# Tech Design: RustRss Android Reader and AI

## Overview

Keep Tauri 2 and the shared Rust core. Add an Android mobile entry point and
platform-specific shell while retaining the existing desktop UI and behavior.
Adapt the static frontend into a phone-first single-column flow. Keep Android
data and AI credentials in platform-managed app storage. Do not build a sync
service or an Android MCP daemon.

## Candidate Designs

1. **One Tauri application with platform-gated shells — recommended.** Share
   `rustrss-core`, Tauri commands, and static frontend; move the application
   builder to a Rust library entry point and gate tray, single-instance, MCP,
   desktop paths, and process helpers by platform. Add a responsive mobile
   layout and Android-specific path/credential adapters. This best reuses the
   current code without duplicating the reader.
2. **Separate Kotlin Android app with Rust core through JNI.** This gives the
   most native Android navigation and lifecycle control, but introduces a
   second UI, JNI contracts, and a separate command surface. It is not needed
   for the requested Android scope.
3. **Web/PWA reader.** This would reduce native build setup, but does not reuse
   the current Tauri command layer directly and complicates secure API-key
   storage and local SQLite ownership. It is not recommended.

## Architecture

### Shared Core and Tauri Shell

- Keep parsing, HTTP feed fetching, SQLite models/queries, search, and AI
  adapters in `rustrss-core`.
- Convert `src-tauri` to produce a shared library with the Tauri mobile entry
  point; keep `main.rs` as the desktop launcher.
- Split setup into shared registration plus platform-gated setup. Android must
  not register the desktop single-instance plugin or initialize tray/MCP
  services. Keep Linux-only GTK/WebKit preview dependencies under Linux cfg.
- Gate child-process utilities and desktop external-open commands. Android
  uses supported Tauri/mobile APIs for URLs and file selection.
- Keep notification plugin registration only if required by existing shared
  code; this release does not promise background-refresh notifications.

### Android Paths and Credentials

- Resolve the app data directory from Tauri's Android path resolver and pass an
  explicit database/log root into the application state/core boundary. Do not
  use `HOME`, `XDG_DATA_HOME`, or process arguments as the Android default.
- The locked `keyring` dependency is v3.6.3 and has no Android native-store
  feature. Evaluate the maintained keyring Android backend in v4 against the
  existing credential API. If it cannot meet the Android lifecycle/build
  requirements, implement a small Tauri plugin backed by Android Keystore.
- Keep provider API keys outside SQLite and app preferences; use the same
  redaction policy in logs as desktop.

### Subscriptions, OPML, and Offline Reading

- Reuse core feed discovery, refresh, import/export, search, and stored article
  content. Use the Android document picker for OPML. Android may return a
  content URI rather than a filesystem path; read/write through the supported
  Tauri filesystem/mobile interface or copy through app cache before invoking
  path-based core code.
- Keep the current SQLite schema unless implementation inspection proves a
  mobile-only migration is necessary. The Android database is new and local;
  OPML is the only subscription transfer mechanism in scope.
- Refresh on startup and foreground resume, plus manual refresh. Disable or
  suspend desktop periodic scheduling while Android is backgrounded. Verify
  lifecycle APIs against the pinned Tauri 2 toolchain rather than assuming a
  desktop window hide event represents app suspension.
- Offline acceptance applies to data already stored in SQLite. An uncached
  full-text fetch still requires a network connection and provider AI calls
  are online operations.

### Mobile UI

- Preserve the current desktop three-pane UI at desktop widths.
- On phone widths, expose feed/navigation, article list, and article reader as
  sequential screens or panels with a clear back path and touch targets.
- Support Android system back and safe-area/status-bar insets. Keep article
  actions available without keyboard shortcuts or right-click menus.
- Keep phone portrait as the primary acceptance layout; ensure tablet widths
  do not clip controls but do not require a separate tablet navigation model.

### AI

- Reuse existing provider adapters and key-source abstraction for
  OpenAI-compatible APIs, Anthropic, Gemini, and Ollama.
- Reuse article summary and translation operations. Keep the current BYOK
  consent/disclosure that article text is sent to the selected provider.
- Ollama is a configured endpoint only; no local model/server is bundled. A
  desktop `localhost` endpoint is not the Android device's localhost, so users
  need an endpoint reachable from the device.

## Data Model

No cross-device sync tables or server account schema are added. Keep the
existing feed/article/settings schema and run the normal schema initialization
for a new Android app-private database. Secure provider credentials remain in
the Android key store, not in settings rows.

## Build and Verification

- Use Tauri's Android target initialization and Rust Android targets with the
  Android SDK/NDK. Produce a signed release APK for sideloading.
- Add Android build/verification steps to CI after a local Android target
  build succeeds; keep existing desktop workflows intact.
- Verify on an Android emulator/device: install/launch, URL discovery, OPML
  import/export, refresh on startup/resume/manual action, search, read-state
  actions, offline access to stored content, AI provider settings, secure key
  persistence, summary/translation, and tablet layout smoke.
- Test Android document content URIs and app sandbox paths; do not treat a
  successful host `cargo check` as mobile verification.

## Module Contracts

- Platform startup supplies an explicit database/log directory before opening
  `AppState`; the desktop launcher continues using the existing desktop path
  rules.
- Article and subscription commands retain core model semantics and errors;
  the mobile UI adapts navigation but does not duplicate storage rules.
- AI configuration uses the existing provider identifiers/settings and a
  platform credential-store adapter behind the current key-source boundary.
- Desktop-only services are not invoked by Android command handlers; disabled
  controls must not imply the MCP service or tray exists on mobile.

## Risks and Mitigations

- **Android secure storage:** keyring v4 is a major-version change. Verify the
  actual Android backend and credential persistence on an emulator before
  committing to the upgrade; otherwise isolate a small native bridge.
- **Desktop coupling in the current entry point:** split shell registration
  behind cfg and retain a desktop build check at each integration step.
- **OPML Storage Access Framework:** test content URIs separately from desktop
  path APIs and keep temporary copies inside app cache.
- **Lifecycle constraints:** only promise refresh on startup/resume/manual
  action; never rely on a suspended Rust timer.
- **Ollama endpoint assumptions:** explain that an endpoint must be reachable
  from the phone; do not silently reinterpret `localhost` as a desktop host.
- **Privacy and sync expectations:** clearly state that OPML moves feeds only;
  reading state, article cache, and AI settings stay on that Android device.

## References

- Tauri project structure and mobile entry point:
  https://v2.tauri.app/start/project-structure/
- Tauri Android SDK/NDK setup: https://v2.tauri.app/start/prerequisites/
- Tauri mobile plugins: https://v2.tauri.app/develop/plugins/develop-mobile/
- Android dialog/content URI behavior:
  https://v2.tauri.app/plugin/dialog/
- keyring Android store feature:
  https://docs.rs/crate/keyring/latest/features
