# 键盘帮助与万篇搜索（2026-09-24）

## 实现与发现

- 原需求的`?`入口确实缺失，新增双语原生dialog，自动聚焦关闭按钮，Esc可退出；弹窗隔离全局文章导航，输入框/设置页不误触发。
- 智能视图和订阅行此前无法Tab聚焦，补role/button与Enter/空格激活、可见焦点框，复用已有点击路径。
- 真实键盘串联发现：侧栏有焦点时j打开文章，随后Enter再次激活侧栏清掉正文。修复为j/k/g/G移动时将焦点交给列表，不重建DOM、不改变滚动位置。

## 证据

`node --test scripts/tests/keyboard-help.test.cjs`：新增4项测试。`?`与背景隔离在实现前真实失败；删除`?`分支的合成负对照确认新测试会红。侧栏按键及嵌套控件隔离、文章导航焦点交接有断言。

重新`cargo build -p rustrss-desktop`后运行：

```sh
/usr/bin/python3 scripts/verify-keyboard-search.py
```

需Xvfb、xdotool、Python websockets和theme_fixture示例。只操作自建Xvfb/隔离库，不触发KDE授权。最新13项断言通过，原始结果见keyboard-search-results.json。

- 真实XTEST Tab聚焦All，空格切换；`?`聚焦关闭按钮、j不改变背景选中行，Esc关闭。
- j/Enter打开已缓存正文；源地址为不可用回环端口，不依赖成功刷新。未禁用主机网络，不宣称完整断网验收。
- 原生`/`与文字输入在10000条夹具中找到正文唯一词对应的Article 5000；接着输入宽泛词并将200条结果渲染到列表，输入事件至DOM更新测得95ms。Esc返回未读列表。
- 同一宽泛搜索的生产search IPC重复5次27–56ms；这些是UI启动后样本，不是零驻留页冷测。SSD严格冷页结果另见`search-ranked-results.json`（core首次65–77ms）；二者不互相替代。
- loopback-only私有网络命名空间复跑16项通过，宽泛搜索UI测得66ms；真实刷新失败后缓存导航继续，10000行保留。原始数据见`offline-namespace-results.json`。
- 新报告夹具将common放入万篇正文，另含中文“中文”“新闻”与稀疏unique词；搜索结果固定最多200条。
- Rust440/0（30段），Node55/0，clippy仅原有3警告，git diff --check通过。

## 未闭合项

键盘全部已读菜单/刷新/所有标志操作串联、标签行焦点、原生Wayland/读屏器尚未全验，因此不勾掉整个键盘AC。完整搜索AC继续开放，等待原生Wayland/release/跨平台性能证据；离线范围未覆盖逐篇打开及外部图片。前两次探针中有夹具updated_at缺失、Inspector顶层let重复声明；另一次捕获真实焦点回归，均已修正后复跑，未把失败运行计通过。
