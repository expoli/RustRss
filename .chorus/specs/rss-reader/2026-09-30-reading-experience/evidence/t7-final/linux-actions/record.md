# T7 Linux desktop action evidence

Run on 2026-10-01 (Asia/Shanghai), in an isolated worktree at `1b4c4ca4489f64a2f929838b86bd843e3eb461ce`. The tested, rebuilt Linux WebKitGTK executable came from main `74de436` and had SHA-256 `706b42fea0d6c18918157264f8d53022ba245bb83dc624ce6f61cb151abfd59f`. The runner used its own `/tmp/rustrss-t7-linux-*/home`, data, config, cache, runtime, SQLite fixture, D-Bus session, Xvfb display `:100`, and private Plasma shell. No user window, session bus, database, wallet, or emulator was used.

## Outcomes

| Matrix item | Result | Direct evidence |
| --- | --- | --- |
| M08.sort, tag drag | Pass on Linux WebKitGTK | Trusted native XTest `dragstart → dragover → drop → dragend`; SQLite `Alpha,Beta,Gamma` became `Beta,Alpha,Gamma` with `sort_order=0,1,2`. [Raw events and rows](results.json), [screen](tag-drag.png), [app log](app.log). |
| M08.sort, folder/feeds | Pass for applicable feed drag | Folder headers expose `draggable=false`; feed rows expose `draggable=true`. Native feed `drop` persisted positions `2→0, 1→1, 3→2`. [Raw events and rows](results.json), [screen](feed-drag.png). No folder-header drag action exists to verify. |
| M16.key-status/save/clear | **Blocked by private Secret Service setup** | `org.freedesktop.secrets` owned, but initial key status was `unavailable`; save and clear returned `Secret Service: no result found`. Synthetic key matched zero SQLite setting rows. Private `kwalletd6` plus `ksecretd` still had no default alias or collections. `CreateCollection` returned an interactive prompt path; the probe did not answer it. [App calls](results.json), [bounded service probe](private-kwallet-results.json), [daemon logs](private-ksecretd.log). This does not establish key save/read/clear acceptance. |
| M16.test | Pass for local failure path only | A second isolated instance with throwaway `RUSTSS_AI_KEY` and `http://127.0.0.1:1/v1` returned a network request failure, with `key_source=env`. The keyring-unavailable failure was also observed in the first instance. [Raw results](results.json). No live provider was contacted. |
| M18.source and reader external URL | Pass through external-launch boundary | Native WebKitGTK reader menu and About source button spawned private `xdg-open` with their exact URLs as separate arguments; `file://` was rejected before launch. [Captured argv](xdg-open.jsonl), [raw calls](results.json). A browser was deliberately not started, so external browser rendering remains unverified. |
| M18.log-level | Pass | `info` produced no `list_scope_total feed#1` DEBUG line; switching to `debug` produced the actual line. SQLite `log.level=debug`. [Raw calls and line](results.json), [file log](app.log). |
| M18.tray and exit | Pass for system effects | Private Plasma StatusNotifier watcher registered the RustRss item and exposed `Show/Hide Window`. `close_action=tray` left the process alive and changed X11 Map State to `IsUnMapped`; the tray menu D-Bus `clicked` event restored `IsViewable`. `close_action=exit` then ended the process with code 0. [Raw states](results.json), [menu layout](tray-menu-layout.txt), [window tree](windows.txt). A physical pointer click on the panel icon was not exercised. |
| M18.log-folder | Pass through external-launch boundary | Directory existed and private `xdg-open` received the exact directory path. [Captured argv](xdg-open.jsonl), [screen](general-actions.png), [raw call](results.json). A graphical file manager was deliberately not started, so its window is unverified. |

The aggregate `results.json` has `passed=false` because M16 key status/save/clear is blocked. All other listed checks passed. AGPL and privacy text are rendered in the About pane; they do not have an external link action in this Linux UI (`ui/index.html`).

## Reproduction

The isolated fixture is [fixture.sqlite](fixture.sqlite). Its URLs and entries are synthetic. From this repository worktree, set `T7_BINARY` to a rebuilt desktop executable and `T7_BINARY_HEAD` to its source commit, then run:

```bash
T7_ROOT=$(mktemp -d /tmp/rustrss-t7-linux-XXXXXX)
mkdir -p "$T7_ROOT/home" "$T7_ROOT/data" "$T7_ROOT/config" "$T7_ROOT/cache" "$T7_ROOT/runtime"
chmod 700 "$T7_ROOT/runtime"
env -u DISPLAY -u WAYLAND_DISPLAY -u RUSTSS_AI_KEY \
  HOME="$T7_ROOT/home" XDG_DATA_HOME="$T7_ROOT/data" \
  XDG_CONFIG_HOME="$T7_ROOT/config" XDG_CACHE_HOME="$T7_ROOT/cache" \
  XDG_RUNTIME_DIR="$T7_ROOT/runtime" \
  T7_BINARY=/path/to/rebuilt/rustrss-desktop T7_BINARY_HEAD=<source-commit> \
  dbus-run-session -- /usr/bin/python3 scripts/verify-reading-experience-t7-linux-actions.py \
  .chorus/specs/rss-reader/2026-09-30-reading-experience/evidence/t7-final/linux-actions
```

For the bounded wallet diagnostic, use a **fresh** private root with the same HOME/XDG/D-Bus setup and run `scripts/probe-reading-experience-t7-private-kwallet.py` in place of the action runner. Its `keyring_probe` command exited 1 with `Secret Service: no result found`; `ReadAlias default` returned `/`, `Collections` returned `[]`, and `CreateCollection` returned `('/', '/org/freedesktop/secrets/prompt/p0')`. No prompt was answered. The actual commands and output are retained in [private-kwallet-results.json](private-kwallet-results.json).

The isolated Xvfb and D-Bus child processes were terminated by the runners. The shell variable above is scoped to the new private root and is never assigned to the user's home.
