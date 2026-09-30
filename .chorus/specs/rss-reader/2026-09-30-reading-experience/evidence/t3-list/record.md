# T3 手机文章、收藏、搜索与列表层次

任务 `9f43e874-2021-406a-9a73-14e28227344a`。使用独立 Android API 36 x86_64 AVD `RustRssT2` (`emulator-5582`) 与只含 `example.invalid` 的隔离库；未操作既有 `emulator-5554`。30 行夹具 SHA-256 `f4674f9df91a1c1fb1927087d93b0052169da0ecc298b782f8b98ff300aa1cba`，200 行夹具 SHA-256 `f338b98364e015a907e5904f92944f5b3d24592915dc49136d20b3e88db4676b`。所有图片为 Android WebView 实际截图，点击由 `adb shell input tap` 完成，返回由 Android Back 完成；CDP 只读取 DOM/状态或注入隔离的请求故障。

重建产物：`src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk`，SHA-256 `2da4907e4fef6fc9448bac2cfeb17a936cfcbdfe301d8f76182b1033accbffd1`，包含 ARM64 + x86_64，`apksigner verify` 通过 v2 签名，证书 SHA-256 `89269c116afaa9ca546f7287b268f00c14d5598b6c633715a74fbbcd28de8cc5`。任务 AVD 执行的是同源码 x86_64 debug APK，SHA-256 `10434282606d4c7bd61c2255f8d47a9e5f9f762025ad565f6c851b38ae07af9c`。

## 同数据改前改后

先在 commit `7e04003` 安装旧 APK，后安装 T3 构建；每轮重新复制同一 30 行库。`360/before-*.png`、`412/before-*.png` 与 `360-threebutton-final/after-*.png`、`412-final/after-*.png` 覆盖中文/英文 × 浅色/深色。JSON 中的坐标由 WebView `visualViewport` 和实际首条标题矩形计算。360×800（三键，WebView 可用高度 776 CSS px）的标题起点从 28.15% 降至 21.59%；412×867（手势）的对应改后采样为 19.32%，原为 25.98%。列表由两行页头收敛为单行分段筛选、计数和动作；手机计数显示 `M/N`，完整本地化口径放在无障碍名称里。长标题、无图行、摘要和图示可从同场景截图直接比对。

## 设备功能与状态

- `functional-360-threebutton-final/results.json` 与 `functional-412-gesture-final/results.json`：每套 5 项通过。四主导航以及未读 15、全部 30、星标 14、稍后读 14 的视图和计数均实际切换；搜索一击展开并显示“搜索范围：全部文章”，真实 IME 缩小 `visualViewport`。查词进入文章、系统 Back 返回保留同一行 DOM；取消后回原全部视图，500px 滚动锚点偏差 <0.4 CSS px。空搜索及隔离故障后的重试也通过。
- `state-200-final/results.json`：6 项通过。200 行全列表、节点身份、三种排序、隐藏已读、当前视图批量已读/未读、刷新结束状态与固定底栏均在真实 SQLite 上检查。故障只拦截下一页 `list_entries` 请求，尾部显示可点的重试；恢复后终止态共 200 行且 ID 无重复。刷新在 `.invalid` 源上如实报告 3 项失败，页首 ID、状态位和底栏坐标不跳。200 行本轮 DOM 列表计时 42.9 ms（仅这台模拟器一次样本，非性能承诺）。
- `viewport-412-gesture-final/results.json`、`viewport-412-font150-final/results.json`、`viewport-landscape-final/results.json` 及截图：四个主导航均在可视区域、目标至少 44 CSS px、无横向溢出。系统字体 1.5 倍时计数完整显示；横屏 WebView 915×364 CSS px 时仍使用四主导航。横屏发现原 900px 阈值会退回桌面布局，已同步调到 960px 并重建验证。`search-ime.png` 保留真实输入法覆盖场景。
- `cargo build -p rustrss-desktop`、61 个 JS 测试和 `git diff --check` 通过。主题渲染的密度、摘要行数、缩略图数据属性与节点保留由共享渲染器测试覆盖；T3 没有改变这些用户设置的存储口径。

## 复现与范围

在任务 AVD 安装 `cargo tauri android build --target x86_64 --debug --apk --ci` 的产物，将隔离 SQLite 复制进应用目录后启动，转发 `localabstract:webview_devtools_remote_<pid>` 到本机 9229。执行 `ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9229/json node scripts/verify-reading-experience-list.mjs <out>`；200 行时改执行 `scripts/verify-reading-experience-state.mjs`。同场景截图由 `scripts/capture-reading-experience-list.mjs <out> before|after` 产生。系统三键/手势、旋转和字体缩放由任务 AVD 的 `cmd overlay`、`settings`、`wm size` 设置；原生可视矩形由 `scripts/verify-reading-experience-viewport.mjs` 留档。桌面最终动作组合由 T6 复核，整体验收由 T7 逐行关闭；本轮未覆盖实体 ARM64、TalkBack 语音、Windows/macOS。
