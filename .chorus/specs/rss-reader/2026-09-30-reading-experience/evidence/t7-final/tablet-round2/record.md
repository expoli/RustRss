# T7 tablet breakpoint grid — final APK

The earlier [tablet captures](../tablet/tablet-results.json) used a prior APK and are **superseded for final-source acceptance**. This grid ran source commit `90f427538a50d946c7318f66585c5bf35f0ff2b4`, final APK SHA-256 `6baf0f068c8a458ee04dee687bb312c3793c0567e2f27ab9bb29956f63053bfc`, on the owned `RustRssT7M15` Pixel Tablet AVD (`emulator-5586`, Android 16/API 36, x86_64, density 320). App data was cleared after the M15 run, so every width showed the same empty-library state.

The [native grid result](tablet-results.json) passed at **768, 849, 850, 851, 959, 960, 961 and 1024 CSS px**. Screenshots [768](tablet-768.png), [960](tablet-960.png), [961](tablet-961.png) and [1024](tablet-1024.png) show the breakpoint transition: mobile bottom navigation through 960 px, three desktop-like panes from 961 px. At every width, document scroll width equaled the CSS viewport width, with no horizontal overflow. The four mobile nav buttons were present at each mobile width; desktop window controls and MCP actions were hidden on Android. At 1024 px, Android OPML import/export controls remained available while backup/restore and MCP controls were hidden. The runner verified the Pixel Tablet hardware profile, physical size and density, WebView `pointer:coarse`, and Android body mode. All eight native screenshots and exact geometry/assertion results are retained with [SHA256SUMS](SHA256SUMS).

The [runner](../../../../../../../scripts/verify-reading-experience-t7-final-tablet.mjs) force-stops and restarts only the owned app per width, uses `adb shell wm size` on the owned emulator, reads native WebView geometry through the inspector, and restores emulator display size/density in `finally`. It does not touch user or main-worker emulators.

```bash
adb -s emulator-5586 shell pm clear tech.expoli.rustrss
ANDROID_SERIAL=emulator-5586 node scripts/verify-reading-experience-t7-final-tablet.mjs \
  /tmp/rustrss-t7-tablet-final-repro \
  /home/tcy/Github/RustRss/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

This is an Android emulator layout check over an empty-library state; populated lists, rotation, hardware touch behavior, and physical tablets are outside this grid.
