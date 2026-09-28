# Task 2ff9d1d3 — Android app-private storage & secure credentials

Device: AVD `RustRss_API_36` (x86_64, API 36, portrait 1080×2400 → CSS 412×839).
Build: `cargo tauri android build --target x86_64 --debug` (probe-free final build).

## What was implemented

1. **App-private storage (AC-1)** — landed earlier in `1631e1d`
   (`rustrss_core::paths::set_data_root` + mobile entry injecting the
   Tauri-resolved sandbox dir before logging/store open). This task verifies it
   end-to-end including restart persistence.
2. **Secure credentials (AC-2)** — new:
   - `src-tauri/src/credentials.rs`: platform split. Desktop = the existing
     keyring implementation (moved verbatim from ai.rs, incl. the
     `keyring_retry` Secret-Service workaround and its tests). Mobile =
     `SecureStorePlugin` bridge (`register_android_plugin` + `run_mobile_plugin`
     get/set/delete).
   - `gen/android/.../SecureStorePlugin.kt`: secret encrypted with an
     Android Keystore AES/GCM key (256-bit, key never leaves the security
     hardware); base64(iv‖ct) stored in app-private SharedPreferences
     (`secure_store.xml`). No Log calls in the class.
   - `ai.rs` keeps `KeySource` and re-exports `store_key/load_key/delete_key` —
     commands.rs and tests unchanged.
   - Response-serialization pitfall found and fixed during verification:
     `resolveObject(JSObject)` is serialized by Jackson as
     `{"nameValuePairs": …}` (org.json internals), losing the payload; the get
     command resolves a plain Kotlin class instead.

## Evidence

| File | Shows |
|---|---|
| `01-restart-articles-persisted.png` | After `am force-stop` + relaunch: DB reopened from `/data/user/0/tech.expoli.rustrss/rustrss/rustrss.sqlite` (log line), saved feed/entries still listed (2 unread rows) |
| `02-ai-pane.png` | AI settings pane before key entry (key: not set) |
| `03-ai-key-saved.png` | (superseded — pre-fix shot; kept for history) |
| `04-ai-key-persisted.png` | After restart: API key hint reads **"Set (from the system keychain)"**, footer **"key: set"** — credential persisted through Android secure storage |
| `leak-checks.txt` | Greps for the test key: 0 hits in SQLite main + WAL, 0 plaintext hits in prefs (file shown: ciphertext only), 0 hits in all app logs; round-trip log line `credentials get: len=28` (length only) |

Repro (save): Settings → AI tab → tap API key → type test key → Save AI settings.
Repro (restart persistence): `adb shell am force-stop tech.expoli.rustrss`
→ `adb shell am start -n tech.expoli.rustrss/.MainActivity` → Settings → AI.

## Desktop regression (AC-3)

- `cargo test --workspace` → 0 failed (incl. the moved keyring-retry tests in
  `credentials::desktop_tests` and the unchanged `crate::ai::load_key` callers).
- `cargo clippy --workspace --all-targets` → clean.
- Desktop code path is cfg-gated to the verbatim keyring implementation; no
  desktop file/behavior change.
