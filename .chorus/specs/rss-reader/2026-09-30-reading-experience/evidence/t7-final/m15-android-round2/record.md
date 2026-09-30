# T7 M15 — final APK native Android rerun

The earlier [M15 record](../m15-android/record.md) used APK SHA-256 `0e011c2f…43091` and is **superseded for final-source acceptance**. This rerun used source commit `90f427538a50d946c7318f66585c5bf35f0ff2b4` and the final debug APK SHA-256 `6baf0f068c8a458ee04dee687bb312c3793c0567e2f27ab9bb29956f63053bfc`. The only device was the owned `RustRssT7M15` Pixel Tablet AVD, `emulator-5586`, Android 16/API 36, x86_64, 2560×1600 at density 320. All feeds and RSSHub probes were local synthetic HTTP through `10.0.2.2`; no live network or user database was used.

## Final pass

The complete [pass-b/results.json](pass-b/results.json) reports `passed: true` and nine checks covering all five requested rows:

| Row | Native observation | Raw evidence |
| --- | --- | --- |
| Global 15 min, feed override 120 min | Both `last_fetched_at` values were aged 30 min. The global feed alone refreshed at the next real 60 s scheduler tick, 67.07 s after arming; override feed was not requested and retained its old timestamp. | `phases.interval`, [interval screenshot](pass-b/interval-after-tick.png), `pass-b/interval-app.log` |
| Refresh concurrency 12 | Android app loaded `refresh_concurrency=12`. A physical Refresh all tap generated a trusted button click and exactly 13 delayed local feed requests; server max-active was exactly **12** with 2 s responses. | `phases.concurrency`, [concurrency screenshot](pass-b/concurrency-after-refresh.png), `pass-b/concurrency-app.log` |
| Cold-start refresh off/on | Separate force-stopped process starts, with interval off: false made no request in 13 s; true made one request 11.898 s after `am start`. Initial Android resume did not make a separate request. | `phases.startup-false`, `phases.startup-true`, [false](pass-b/startup-false.png), [true](pass-b/startup-true.png), app logs |
| Notification positive and manual negative | `dumpsys package` confirmed `POST_NOTIFICATIONS` granted. HOME then reentering the app caused background/foreground-resume refresh, unread 0→1, one app notification log and one Android notification record with `android.text=1 new article`. The [expanded notification shade](pass-b/notification-shade-expanded.png) visibly showed it. Native Refresh all then inserted a second unread article (1→2), with the same active notification ID and no second notification log line. | `phases.notifyBackground`, `phases.notifyManual`, `pass-b/notification-*-app.log`, `pass-b/notification-*-notification.txt.gz`, [manual screenshot](pass-b/notification-manual-control.png) |
| RSSHub unavailable | Native Test connection button probed `/version`, `/`, `/rsshub/rss`, `/feed/rsshub/rss`; each local route returned 503 and UI reported no 2xx route. | `phases.rsshub`, [screenshot](pass-b/rsshub-503.png), `pass-b/rsshub-app.log` |

The first [attempt-a-inconclusive/results.json](attempt-a-inconclusive/results.json) completed the interval check and loaded concurrency 12, but its 0.8 s local response delay exposed only **11** simultaneously active requests from 13 total. It stopped at the stricter saturation assertion. That result neither established all 12 lanes nor showed a cap violation; it is retained as an inconclusive harness attempt. The second run changed only the synthetic response delay to 2 s and observed all 12 lanes. The raw first-attempt fixture, interval screenshot, logcat and notification dump are retained. Android logcat and notification dumps for both runs are byte-preserving gzip files.

## Reproduce and limits

The [runner](../../../../../../../scripts/verify-reading-experience-t7-m15.py) requires the owned AVD on serial `emulator-5586`, an installed APK with the exact hash, Android SDK tools, system Python `websockets`, and the committed 200-row SQLite schema fixture. It prepares phase-specific SQLite fixtures on the host, pushes isolated copies into the app, serves local RSS and 503 responses, then uses Android activity starts, HOME, physical taps, CDP readback, SQLite, `dumpsys`, app logs, logcat and native screenshots. `pass-b` includes every phase input fixture and [SHA256SUMS](SHA256SUMS) hashes all retained raw files.

```bash
adb -s emulator-5586 install -r /home/tcy/Github/RustRss/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
adb -s emulator-5586 shell pm grant tech.expoli.rustrss android.permission.POST_NOTIFICATIONS
ANDROID_SERIAL=emulator-5586 /usr/bin/python3 scripts/verify-reading-experience-t7-m15.py \
  --apk /home/tcy/Github/RustRss/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk \
  --expected-apk-sha256 6baf0f068c8a458ee04dee687bb312c3793c0567e2f27ab9bb29956f63053bfc \
  --out /tmp/rustrss-t7-m15-final-repro
```

This is emulator evidence. It does not establish physical-device notification delivery, real RSSHub availability, doze reliability, or a 15-minute wall-clock run. The pre-aged interval rows test policy at the next real 60-second tick. Startup timing is measured from the `am start` command. The manual negative control shows that a successful manual refresh with a new unread item did not emit an additional notification.
