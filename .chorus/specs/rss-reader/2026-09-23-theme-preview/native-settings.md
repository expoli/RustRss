# 原生 Wayland 分数缩放与系统明暗验收（2026-09-24）

基线 `aabbfe6`。本轮仅增加验收脚本和文档，没有产品代码修改。

## 范围与前置条件

用户明确允许临时切换外屏缩放和系统配色，完成后恢复。KDE Wayland 真会话，外屏 HDMI-A-1 / 2560×1440；原始外屏110%、内屏150%、BreezeLight。没有 Xvfb、CSS zoom、伪造 matchMedia 或指针授权弹窗。

使用独立数据库启动 RustRss，实际产物晚于上轮 UI 修改，不重复编译。脚本按隔离PID定位和移动窗口；通过回环 WebKit inspector 读取状态和执行自动化操作，真实点击另由用户完成。未读取用户订阅库，也未截取整个桌面。

## 结果

| 验收点 | 125% | 150% |
|---|---|---|
| KScreen 原生输出缩放、隔离窗口位于外屏 | 通过 | 通过 |
| 主窗口1240×820，无水平溢出 | 通过 | 通过 |
| 设置弹窗在视口内 | 通过 | 通过 |
| MCP截图后端 GdkWaylandDisplay、文件可读 | 通过 | 通过 |
| 设置→阅读真实点击，isTrusted=true且坐标在按钮内 | 通过 | 通过 |
| 文字/图标清晰、布局正常 | 用户确认 | 用户确认 |

两档 WebView `devicePixelRatio=2`，MCP PNG逻辑尺寸960×640、像素1920×1280。GTK整数缓冲比例和KDE输出分数缩放是不同层；这里用 KScreen 与 KWin 窗口所属输出证明125%/150%，未把 DPR2 写成200%桌面缩放。清晰度来自用户肉眼确认，不是由PNG尺寸推断。

系统明暗四组实测：

| 应用模式 | 系统配色 | 实际media查询 | 应用结果 |
|---|---|---|---|
| system | BreezeDark | dark | dark，背景#14161a |
| system | BreezeLight | light | light，背景#f7f8fa |
| light | BreezeDark | dark | 保持light |
| dark | BreezeLight | light | 保持dark |

四组均为真实系统设置变化驱动现有WebView；OS切换前后SQLite正式主题配置完全相同，文章DOM节点保留、段落偏移均为0.09375px。自动部分20项断言通过；人工部分两档共4次按钮点击通过；无产品缺陷被复现。

## 复现

仅在用户允许改变桌面全局设置、屏幕保持解锁时运行：

```bash
/usr/bin/python3 scripts/start-theme-wayland-probe.py
# 使用启动器打印的 instance.json 路径；以下 INSTANCE_JSON 为该实际路径。
/usr/bin/python3 scripts/verify-theme-native-settings.py --allow-desktop-changes INSTANCE_JSON
/usr/bin/python3 scripts/verify-theme-native-settings.py --allow-desktop-changes --manual-scale 1.25 INSTANCE_JSON
/usr/bin/python3 scripts/verify-theme-native-settings.py --allow-desktop-changes --manual-scale 1.5 INSTANCE_JSON
```

需要 KDE 的 kscreen-doctor/plasma-apply-colorscheme/qdbus6/journalctl、Python websockets；默认目标HDMI-A-1，且目标输出必须位于布局原点。人工档等待180秒，点隔离RustRss的“设置”→“阅读”；成功或异常均进入finally恢复原缩放和配色。强杀Python/断电无法承诺finally运行，必要时用 `kscreen-doctor output.HDMI-A-1.scale.1.1` 和 `plasma-apply-colorscheme BreezeLight` 恢复本次原值；其它机器须使用各自记录的原值。

探针首轮误传了接口不支持的width/height字段，被拒绝；恢复逻辑正常执行。核对PreviewParams的deny_unknown_fields后删除这两个探针参数，完整重跑通过，未修改产品API。

机器证据：[结果JSON](native-settings-results.json)。Python语法检查通过，脚本实际执行通过；无Rust/UI产品修改，本轮未重复cargo/Node全量测试或构建，上轮413/42结果只作历史参考。

## 收尾与限制

- 已独立回读确认外屏110%、内屏150%、位置保持原值、系统BreezeLight；已通知用户可以锁屏。
- 两档清晰度为用户主观确认，不是量化锐度基准；未验证175%、其它显示器/显卡、X11、Windows/macOS。
- 本轮覆盖主阅读界面、设置弹窗、固定预览和所列按钮；没有穷举每个弹窗或控件。
- 系统主题测试用默认Clear配色，不重复三预设全矩阵。未改变用户正式主题或数据库。
- 隔离桌面已通过exit_app退出，启动器EXIT=0；本轮临时目录及生成的pycache已清理，历史暂留产物未动。磁盘余量约54G（94%）。
- 验收完成时未执行commit/push；后续按用户指示单独提交。
