# Task 2b777b1c — mobile navigation evidence record

Device: AVD `RustRss_API_36` (x86_64, API 36, portrait 1080×2400 @ 420 dpi →
CSS viewport **412×839**, `dpr=2.625` — see `viewport` line in the app log).
App: `app-universal-debug.apk` built by `cargo tauri android build --target x86_64 --debug`.

## Screenshots (portrait phone)

| File | Shows |
|---|---|
| `01-articles-launch.png` | Launch lands on **Articles / unread**; bottom nav Articles · Subscriptions · Saved · Settings; unread/all segment; search + refresh in page; system status bar outside app content (insets handled by MainActivity) |
| `02-subscriptions-empty.png` | Subscriptions page (Add feed visible; search/refresh hidden) |
| `03-articles-with-entries.png` | (superseded by 04 — taken right after feed add) |
| `04-articles-from-subscription.png` | Tapping the feed row on Subscriptions switched to **Articles filtered to that feed** (title = feed name, "loaded 6 / 6 total") |
| `05-articles-scrolled-before-reader.png` | List scrolled to a distinctive offset (第一篇 clipped at top, End of list visible) |
| `06-reader-fullscreen.png` | Full-screen reader for 第二篇; **Back to list** button; touch actions Aa / Mark unread / Star / Read later / Open in browser / Copy link / Get full text / AI summary / AI translate; bottom nav hidden (immersive) |
| `07-articles-after-back.png` | After `input keyevent 4` (Android Back): same destination, same filter, **same scroll offset**, 第二篇 selected + marked read; nav restored |
| `08-reader-starred.png` | Reader after tapping Star — button flipped to **Unstar** |
| `09-saved-starred.png` | Saved destination, **Starred** segment active, 第二篇 listed (1 / 1) |
| `10-saved-readlater.png` | Same page, **Read later** segment active, 第四篇 listed (1 / 1) — one switch, two views, no discovery destination |
| `11-settings-appearance.png` | Settings as full-screen page with horizontal tabs; no MCP tab |
| `12-settings-tabs-end.png` | Tab bar scrolled to the end — last tab is **General**; MCP tab removed on mobile |
| `13-settings-general.png` | General pane: language, about, log level — **no close-action (tray) control, no open-logs-dir button** |
| `14-subscriptions-longpress-menu.png` | Long-press on a feed row opens the management menu (Refresh now / Edit / Refresh interval / Move to / Unsubscribe) |

## Recorded Back-restoration interaction (AC4)

Reproducible command sequence on the booted AVD with the app in the foreground:

```bash
adb shell input swipe 540 1900 540 700 400   # scroll the Articles list
adb exec-out screencap -p                    # 05: pre-reader scroll state
adb shell input tap 540 830                  # tap 第二篇 row → full-screen reader
adb exec-out screencap -p                    # 06: reader
adb shell input keyevent 4                   # Android Back
adb exec-out screencap -p                    # 07: restored list
```

App-log proof that the list was never rebuilt (restoration is the untouched DOM,
not a re-render): between opening and closing the reader there is **no
`renderList` line** — only the reader render and the read-mark:

```
14:05:19 [ui] renderList rows=6 withTags=0 2.5ms          ← list built (before scroll)
14:05:56 [ui] renderReader id=2 sanitize=0 innerHTML-set=1 highlight=1 total=1ms
14:05:56 [ui] open id=2 markRead=true read=false          ← reader opened (pushState)
   … Android Back pressed …                               ← popstate: reader closed,
14:06:55 [ui] renderReader id=2 … (re-open for star)         page restored, zero list work
```

Screenshots 05 and 07 show the identical scroll offset (第二篇 at the same y;
第一篇 clipped identically) with 第二篇 highlighted/selected.

## Mechanism notes

- Android Back reaches the web app because wry's `WryActivity` calls
  `webView.goBack()` when `canGoBack()`; the reader/overlay open paths call
  `history.pushState`, so Back arrives as `popstate` and the module restores the
  originating page. At a primary destination the history stack is empty, so Back
  does not cycle destinations (Android default: the app exits).
- The list page is kept rendered under the reader cover (no `display:none`),
  which is why filter, selection and scroll position survive exactly.
- Insets: the generated `MainActivity` calls `enableEdgeToEdge()`; system-bar +
  display-cutout insets are applied as padding on `android.R.id.content`
  (MainActivity.kt), keeping app content outside the status bar and gesture area.
