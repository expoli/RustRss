# T7 独立评审 P2 收尾

2026-09-24，基线 `8d386e2`。用户确认 fresh reviewer 的 VERDICT 通过；本次仅消化两项 P2，不关闭 T7 原生 Wayland 输入 open item。

## A：删改依据与行为边界

`ui/index.html` 已没有 `set-font-size`、`set-font-line`、两个 `*-value`、`set-font-ui/read/mono`、`set-theme-preset`，ui 内也没有创建路径。原绑定函数取不到节点即 return；旧 tooltip/标签刷新同样为空操作，旧 preset 不在 `SETTING_DROPDOWNS` 中。

因此删除旧控件专属的 `paintFontControls` / `paintFontValueLabel`、`bindFontControls`、`syncFontListHints`、无调用者的 `fontChoices`，以及调用处和 preset 错误处理死分支。旧 `fontPreview` 的唯一写入者位于不可达的旧 input/change 回调中，原值始终为空；删除空覆盖与重复 `applyFontConfig` 包装，仍由原来的 `applyTheme` 应用相同 snapshot。

保留项及替代路径：

- 后端 `set_font_config` command、`set_font_config_core` 及兼容测试未改；只删除前端不可达的旧滑块 IPC 回调。
- `mountThemeEditor` → `RustRssThemeSettings.createEditor` 仍供外观、阅读、Aa 共用。`validate_ui_theme` 做草稿局部预览，`update_ui_theme` 保存；`paint` 更新数值输入及示例，`acceptThemeSnapshot` 刷新编辑器。这些代码未改。
- 字体族预取与缓存、共享编辑器的 `fontNames`/datalist 保留。
- `clampNumber`、`fontSizeText`、`fontLineText` 仍用于启动日志，保留；一般设置下拉刷新和错误提示保留。
- 未修改参数范围、持久化、默认值、用户交互或后端接口。

## B：契约与变异校验

`node --test scripts/tests/preview-contract.test.cjs` 包含7个测试；使用纯函数和内存合成坏输入，不改仓库文件。

- 递归扫描全部 `ui/**/*.js`，包含 vendor。支持当前字面量 lookup、三元标签、ID数组、bind、dropdown descriptor/comparison 和 ID selector 形式。
- ID 必须在 markup 或 ui 的 HTML模板 / `.id =` / `setAttribute('id', ...)` 创建路径出现；不对白名单前缀 `act-*`、`ai-panel*` 等整段放行。`data-id` 不算 DOM ID。
- 子资源解析不依赖属性顺序、单双引号、`defer/type` 或换行，并覆盖 stylesheet link。协议侧按实际 match arm 检查；先应用实际入口字符串替换，不能假设任意 `app.js` 写法均会自动改成 preview。
- ID negative control：向每个 UI JS 注入已删除 `set-font-size`，逐一断言失败；另有10种独立旧引用/任意不存在ID样本，以及去除动态创建路径的反例。
- 资源 negative control：增加未服务JS、CSS；从内存协议样本移除 `theme-settings.js` / `style.css` 路由；改变入口引号使现有替换失效。各例均断言契约报错。
- 修复前实跑新断言，准确列出 app.js 的8个旧ID，1项测试红；清理后7项通过。整个Node测试集40项通过。

**限制**：这是针对当前代码写法的静态契约扫描，不是完整JS AST/控制流证明。计算ID（如 `'tab-' + pane`）不穷举，创建路径存在不等于所有时序下节点必在；注释/字符串不是完整词法隔离。HTML属性扫描面向本仓库静态标签，不解析实体及引号内的 `>`。若引入新写法，需扩展扫描与变异样本；运行时验证仍不可省略。

## 验收

- `cargo test --workspace`：405 passed /0 failed，26结果段，exit0。
- `node --test scripts/tests/*.test.cjs`：40 passed /0 failed，exit0。
- `cargo clippy --workspace --all-targets`：exit0，仅既有 `fulltext.rs:130`、`store/mod.rs:478/697` 三条告警。
- `node --check ui/app.js` 通过。
- `cargo build -p rustrss-desktop`：exit0，13.32秒增量重建并重新嵌入本轮UI。
- `python3 scripts/verify-theme-desktop.py`：重建后Xvfb单档，七类键盘导航、焦点回环、Aa草稿零写、23px/revision1保存、重启保持，正文渲染1→1，exit0。

[机器结果](p2-contract-cleanup-results.json)。没有重复截图矩阵/30分钟长测，也没有触发KDE授权；Wayland真实点击、其它平台、真实第三方MCP客户端等原有未验证项保持原状态。

## 磁盘与收尾

动手前62G可用（94%）；重建/probe前分别检查，probe前约61G。只生成本轮隔离目录和 `/tmp/rustrss-p2-*` 日志；不改旧T4/T7证据或Cargo缓存。

单档probe正常退出；已清理本轮 `/tmp/rustrss-theme-desktop-zoi7tmzz`（约4.8MiB）及5份本轮日志，机器摘要已入库。使用字面路径、未使用force或绕过guard。未生成/提交 `__pycache__`、临时PNG或日志。
