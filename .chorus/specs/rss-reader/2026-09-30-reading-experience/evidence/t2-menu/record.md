# T2 订阅、文件夹与标签操作面板

任务 `17a5a3e7-c1b6-447f-a880-cdf44e2a37b4`；产品源码提交见本文件后续提交历史。隔离夹具来自 `reading_experience_fixture`，所有写入只发生在任务专用模拟器和临时桌面数据库。M03–M09/M20 的逐动作状态见 [动作台账](../../action-matrix.md)。

## 产物与复现

| 产物 | 路径 | SHA-256 | 范围 |
| --- | --- | --- | --- |
| Android 通用 release APK | `src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk` | `811d039715f8c824f71830aacc4ec707770bd8536667cc4c829e9c16ddeca5a9` | 73,063,542 字节，ARM64 + x86_64，0.2.1，签名证书 SHA-256 `89269c116afaa9ca546f7287b268f00c14d5598b6c633715a74fbbcd28de8cc5` |
| Android x86_64 debug APK | `src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk` | `b9596a88aef98887ecfb0d41d636c43143deeec1720175dc7e6432d46d904cb9` | 下列设备自动检查使用此包 |
| Linux 桌面 debug 二进制 | `target/debug/rustrss-desktop` | `9d8e7d39d5fb8dd9adfddb102a85f1c0a7f265b45711dc666c79a8fe039648a1` | 虚拟 KWin Wayland 会话内的 WebKitGTK |

复现设备步骤：创建独立 API 36 Google APIs x86_64 AVD `RustRssT2`，启动在 `emulator-5582`；运行 `cargo tauri android build --target x86_64 --debug --apk --ci` 并安装 debug APK；通用签名包使用 `cargo tauri android build --target aarch64 x86_64 --apk --ci` 重建。用 `cargo run -p rustrss-core --example reading_experience_fixture -- /tmp/rustrss-t2-fixture.sqlite 30` 生成隔离库，再执行下面 SQL。将数据库放到该 AVD 的应用数据目录。以 `adb forward tcp:9228 localabstract:webview_devtools_remote_<app-pid>` 暴露 WebView Inspector，执行 `ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9228/json node scripts/verify-reading-experience-menu.mjs evidence-dir`，随后运行 `ANDROID_SERIAL=emulator-5582 node scripts/verify-reading-experience-menu-restart.mjs evidence-dir`。脚本使用 Android 原生命中与系统 Back，同时通过 WebView Inspector 查 UI 状态、通过实际 Tauri IPC 回读对象。重启应用并刷新 WebView forward 后可执行 `ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9228/json node scripts/record-reading-experience-menu-talkback.mjs evidence-dir`；它只在任务 AVD 启用 TalkBack。桌面执行 `cargo build -p rustrss-desktop`，然后 `dbus-run-session -- /usr/bin/python3 scripts/verify-reading-experience-menu-desktop.py evidence-dir`；该脚本从隔离库启动真实桌面 WebKitGTK 组件。

```sql
INSERT INTO tags(id,name,pinned,sort_order,created_at) VALUES
  (1,'Tag A · 设计与阅读',0,0,1), (2,'Tag B',0,1,1), (3,'Pinned',1,2,1);
UPDATE feeds SET folder_id=1 WHERE id=1;
```

## 验证结果

- [Android 结果](results.json)：40 项通过。行内“更多”由原生坐标点按，菜单保留原源 ID；刷新间隔与移动目标回读；源上下移、文件夹与标签折叠、标签颜色/默认色、非拖拽上移与下移、置顶/取消置顶/重命名、文件夹与源编辑、取消订阅、删除文件夹/标签均执行并回读。文件夹删除取消无写、确认后源保留；取消订阅与标签删除取消无写、确认后生效。发现两个候选源后选择第二项，新源 URL 及单源刷新成功状态回读；无效地址失败保留输入。文章标签选择器中英文搜索、选择、新建、附加、移除和 chip 跳转已测。
- 审查补证：M04.rsshub 使用完整输入 `rsshub://t2/menu-fixture`。设备上的 `set_rsshub_mirror` 指向脚本在宿主机临时启动的 `http://10.0.2.2:41059`，隔离服务器收到 `/t2/menu-fixture`，返回本地 RSS XML；手机添加入口执行后回读到源 ID 4、存储 URL `rsshub://t2/menu-fixture`、标题 `T2 RSSHub fixture`、`last_status=ok`（[结果 JSON](results.json) 的 `rsshub-full-address-add-result`）。端口为该次运行动态分配；复跑以结果 JSON 为准，不使用外部 RSSHub 服务。另在原生 Android 点按确认/取消后核焦点：文件夹删除取消回原更多、确认后落到存活的 `h:2`；取消订阅取消回原更多、确认后落到 `f:1`；标签删除取消回原更多、确认后落到 `t:1`。三项确认后原行均不在 DOM，焦点目标有可见区域；这些结果分别在 `folder-delete-*`、`unsubscribe-*`、`tag-delete-*` 检查中。没有可见相邻行时实现会聚焦列表标题。
- 原生 750 ms 长按先打开一份面板；系统 Back 可关闭它，紧接着点击源行仍能导航。WebView 自身追加的重复 `contextmenu` 已去重，且面板关闭会清理长按的点击抑制标志。系统 Back 在手机选择页先返回操作列表，再关闭面板；无写入且焦点回到原“更多”按钮。菜单按钮与条目高至少 48 CSS px；360 与 412 CSS px 的实际 Android WebView 尺寸无横向溢出。输入法弹出时订阅地址输入框位于 `visualViewport` 内，见 [IME 截图](06-add-ime.png)；长菜单采用内部滚动，见 [文件夹选择页](02-folder-long-options.png) 和结果 JSON 的 `folder-options-internal-scroll`。截图：[长源标题](01-feed-long-title.png)、[间隔当前值](02-interval-current.png)、[标签当前颜色](03-tag-color-current.png)、[删除影响确认](04-folder-delete-impact.png)、[发现候选](05-discovery-candidates.png)、[360](05-menu-360.png)、[412](05-menu-412.png)。
- [重启结果](restart-results.json)：操作后关闭并重新启动任务 AVD 中的应用，标签区仍折叠；再次展开成功；源 2 的自定义名和 30 分钟间隔、标签 2 的取消置顶状态均仍在。
- [桌面结果](desktop-results.json)：真实 Linux WebKitGTK 在隔离虚拟 KWin Wayland 会话通过 11 项。菜单首项焦点、上下方向、右键进入子菜单、左键返回、Esc 关闭与焦点恢复，以及打开/关闭不写库均确认。测试键由 Inspector 向真实 WebKit DOM 发送 `KeyboardEvent`；没有声称为操作系统级实体键盘注入。
- TalkBack 在同一任务 AVD 启用时，`dumpsys accessibility` 显示服务已启用（[服务状态](talkback-service.txt)）。原生 [行无障碍树](talkback-rows.xml) 将 `T2 edited feed` 选择按钮与 `更多操作：T2 edited feed` 分成独立 `android.widget.Button`；[间隔选择页无障碍树](talkback-interval.xml) 有命名的返回/关闭按钮、`android.app.Dialog`、`android.view.MenuItem`，其中“每 30 分钟” `checkable=true, checked=true`，背景由 `inert` 隔离（[DOM 记录](talkback-dom.txt)）。对应 [行截图](talkback-rows.png) 与 [选择页截图](talkback-interval.png)。这里只核查服务与原生语义树，没有观测语音或盲文输出。

## 范围

设备运行仅覆盖任务拥有的 Android 36 x86_64 模拟器；通用 release APK 已重建、签名和核对 ABI，但未在物理 ARM64 手机上运行。桌面实测为虚拟 KWin Wayland 的 Linux WebKitGTK；Windows/macOS 与实体键盘由后续整体验收覆盖。动作台账只保留 M08.sort 的桌面旧拖拽最终组合复核，由 T6 负责；T2 手机更多菜单上下两向均已执行并回读。其它非 T2 责任动作和跨端整体验收由后续任务逐行关闭。
