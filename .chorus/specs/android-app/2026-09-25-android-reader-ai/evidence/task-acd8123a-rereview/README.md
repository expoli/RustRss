# Task acd8123a independent re-review evidence

Captured on 2026-09-27 against commit `53a78b7` using AVD `RustRss_API_36`
(Android 16 / API 36 / x86_64).

## Android build and runtime

- `android-build.log`: successful
  `cargo tauri android build --debug --apk --target x86_64 --ci` output.
- `adb-install.txt`: `adb install -r .../app-universal-debug.apk` returned
  `Success`.
- `am-start.txt`: cold launch of `tech.expoli.rustrss/.MainActivity` returned
  `Status: ok`.
- `pid.txt`: live application PID after launch.
- `dumpsys-activity.txt`: `MainActivity` is the top resumed activity.
- `dumpsys-window.txt`: application window exists and is focused.
- `android-launch.png`: 1080x2400 framebuffer screenshot showing the RustRss UI.
- `logcat.txt`: device log captured after launch; no RustRss fatal exception or
  process-death event and no desktop MCP/tray/single-instance startup line.
- `app-files.txt`: application-private files include the RustRss database and log.
- `app-log.txt`: application log records the database and log under
  `/data/user/0/tech.expoli.rustrss/rustrss/`, successful UI self-tests and render.
- `sha256.txt`: screenshot and installed APK hashes.

## Desktop regression

- `cargo-test-workspace.log`: `cargo test --workspace`, exit 0; all suites passed.
- `cargo-clippy.log`: `cargo clippy --workspace --all-targets`, exit 0, no warnings.
- `cargo-build-workspace.log`: `cargo build --workspace`, exit 0.

The screenshot was also visually inspected during review. It shows the current
shared desktop-oriented three-column UI rendered on the phone-sized Android
framebuffer; adapting that UI is explicitly owned by the separate phone-navigation
task, not this application-entry-point task.
