# 文件协议脚本与 UI 体验检查（2026-09-24）

用户选择以脚本替代模型做接口验收，并继续 UI 体验检查。本轮不调用模型读图，不宣称解决近似图片连续读图异常。

## 可复现命令

```bash
df -h .
cargo build -p rustrss-desktop
cargo build -p rustrss-core --example theme_fixture
/usr/bin/python3 scripts/verify-theme-preview-files.py
cargo build -p rustrss-desktop --features snapshot-probe --example theme_ui
/usr/bin/python3 scripts/verify-theme-settings.py
/usr/bin/python3 scripts/verify-theme-desktop.py
```

需 Linux、Xvfb、xdotool、ImageMagick 与 distro Python Pillow；从仓库根运行。每个脚本使用隔离数据库，不改用户主题。Xvfb 子进程剔除会话的 GDK_BACKEND/WAYLAND_DISPLAY/EGL_PLATFORM，产品代码不设置后端。文件验收脚本模拟 MCP 客户端，但使用真实桌面服务、PNG、SQLite 和600秒时钟；保存/取消后不再发请求，观察后台独立回收。每分钟记录桌面主进程 RSS 与磁盘余量，不代表整个 WebKit 进程树内存。

## UI 修复与证据

- 字体列表异步返回时，以前只更新缓存；已聚焦输入框的 datalist 不刷新。增加共享编辑器 refreshFonts，加载完成通知外观、阅读与 Aa；不重建编辑器，不重置草稿或输入文字。
- 字体加载失败原来永久保留 loaded 标志；现在清除标志，下次打开重试，同时保留进行中去重。
- scripts/tests/theme-settings.test.cjs 两条复现测试在旧代码上分别得到 `[]`（应通知三个编辑器）和调用次数 `1`（应重试为 `2`）；修复后通过。运行夹具另断言已关闭编辑器不会被晚到更新修改。
- WebKit 夹具23项断言/3张截图通过，检查双语空阅读区、系统偏好事件切换、不写配置、注销监听器、字体建议保留焦点/文字/草稿、Aa 字体栈保存、125%/150% CSS zoom 下字段容器不溢出；原有保存/CAS/历史恢复/正文节点与位置断言继续保留。
- 真实桌面 Xvfb 键盘：七分类、焦点循环、Aa 草稿不写、保存后重启保留、正文渲染次数1→1通过。
- cargo test --workspace：413 passed / 0 failed（26段）；Node：42 passed / 0 failed；clippy仅原有3条core警告；桌面及 probe 示例重建后才运行验证。

## 范围与未验证

- 空状态本轮覆盖双语空阅读区，未穷举无订阅、网络错误、空搜索等全部业务状态。
- 系统偏好以注入 media 对象触发真实 renderer 回调；没有修改用户桌面明暗设置。
- 125%/150% 是 CSS zoom 布局压力检查，不等于原生 Wayland 分数缩放；原生档位仍待专门环境。
- 字体测试证明建议/字体栈传递与保存，不推断每个字形实际来自哪种字体，也未验证系统字体建议弹窗的原生导航。
- 本轮未重复 Wayland 真实点击、Windows/macOS、全矩阵或30分钟长测；既有结果仅作历史参考。
- 文件脚本正常结束前清理本实例遗留 PNG；SIGTERM 停止夹具不算 Tauri 正常退出证据，正常退出清理见前轮报告。

## 真实600秒文件回收结果

脚本 EXIT=0，8项契约断言全部通过，3张 PNG。首图请求开始后的602.51秒采样三文件仍在；605.01秒已全部由后台回收。每张图在自身请求开始后的前600秒持续检查未提前删除，保存与取消均保留文件直至过期；等待期间没有 MCP 请求，正式配置最终与保存后基线相同（revision=1、Paper）。时间包含生成/重拍间隔与5秒清理周期，未缩短 TTL 或伪造时钟。

桌面主进程 RSS 起始210980 KiB、结束204992 KiB，等待期稳定，无持续上升；不包含 WebKit 子进程。磁盘初始55G、验收采样末尾54G、收尾时55G可用（94%），中间并行执行增量构建，不能把约1GiB差额归因于预览；截图只有3张，构建产物未清理。

机器证据：[完整结果](file-probe-ui-followup-results.json)。UI临时目录已清理；最后一批清理命令被本机守卫拒绝（rm -f style commands are not permitted），没有绕过。暂留 `/tmp/rustrss-preview-files-vyqxv35c` 与 `/tmp/rustrss-ui-followup-*.log`；正式PNG已由服务过期回收，暂留目录560KiB、日志约64KiB。验收完成时尚未执行 commit/push；后续按用户指示单独提交。
