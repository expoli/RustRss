# 阶段③：流式与体验 · 2026-10-05

基线 HEAD `6dfef8d`（阶段②与安全修复之后）。只做四家模型文本流式、历史分页、增量 UI 与无障碍；不扩展工具权限或工具执行流式，不引入前端构建链。`mobile.js` 的既有显式容器/日报分段继续复用，未改路由。

## 机械证据：已验证

- `cargo test --workspace`：**610 passed、1 个既有 ignored**（588 + 22），见 [workspace.log](workspace.log)。新增 parser 5、传输 12、streaming agent 4、store 分页 1。
- `cargo clippy --workspace --all-targets -- -D warnings`：**零警告**，见 [clippy.log](clippy.log)。
- `cargo build --workspace`：成功，见 [build.log](build.log)。
- `node --test scripts/tests/*.test.cjs`：**89/89**（83 + 6），chat-ui **26/26**，见 [js.log](js.log)。
- Rust 覆盖：每个字节位置切 UTF-8、多工具乱序 index 拼参、显式 SSE id 重复、合法重复 token 不误删、迟到文本/usage、空 chunk 非 EOF、无终止符/截断最后事件、四家成功+断流、OpenAI include_usage 明确拒绝才重试/未知 usage、真正 HTTP chunk 一字节分包及提前回调、体积闸门/超时、四家工具循环（Gemini 签名原样回放）、123 条历史无缺口分页。
- JS 覆盖：120ms 合并渲染、assistant/用户节点身份不变、上滚不滚底/近底跟随、回执前与隐藏路由文本保留、跨 session/message/seq 与终态去重、分页 single flight/去重/高度锚点/迟到回执、刷新保留已加载旧页、失败前保留 live 气泡且错误提示不被部分回答覆盖。

## 桌面 Xvfb 真产物：已验证

使用隔离 `/tmp/rustrss-stage3-smoke/home`、合成 `db.sqlite`、独立 Xvfb 与 loopback Ollama mock（无 key）。未修改 GDK_BACKEND/QT_QPA_PLATFORM；隔离 XDG_RUNTIME_DIR 避免接入宿主 Wayland，`GDK_GL=disable` 只用于无头渲染。动作来自 WebKit inspector DOM，不冒称原生指针或键盘操作。

证据：[xvfb/evidence.json](xvfb/evidence.json)、[应用日志](xvfb/app.log)、[实际冒烟脚本](xvfb/smoke.py)。脚本只对固定 `/tmp/rustrss-stage3-smoke/db.sqlite` 合成库操作（重新运行前须先用当前桌面二进制初始化该库/目录），使用仓库已有 Probe 与系统 `/usr/bin/python3` websockets。产品代码没有测试钩子。

1. [01-streaming.png](xvfb/01-streaming.png)：真实 `chat_send` → mock NDJSON，未结束时部分回答/「正在接收回答…」可见，停止 enabled；polite log 与输入标签 DOM 断言通过。
2. [02-streaming-upscroll.png](xvfb/02-streaming-upscroll.png)：确认消息内容实际溢出后上滚到 0，后续文本增长、assistant 与 user 节点身份不变，scrollTop 保持 0。
3. [03-complete.png](xvfb/03-complete.png)：60 段完整回答结束；真实 SQLite 回读 status=done、input=11/output=600，持久正文逐字等于全部增量，chunk seq 单调递增。
4. [04-pagination.png](xvfb/04-pagination.png)：合成 122 条历史，通过真实 `chat_session_get` IPC 先读最新 50，上滚 prepend 到 100；末尾节点保持，scrollTop 用高度差补偿（2750px）。
5. [05-disconnected.png](xvfb/05-disconnected.png)：mock 发送 10 段后无 done 关闭；已有文本保留、EOF/81 字符错误可见、草稿恢复；SQLite status=failed，部分文本与错误均落库。

五张截图 damage 后哈希不同；二进制 mtime 晚于 app.js，SHA-256 见 evidence.json。截图只包含合成资料。

## 协议差异 / 容错口径

| Provider | 帧与终止 | 工具/usage |
|---|---|---|
| OpenAI 兼容 | SSE data；必须 `[DONE]`，finish_reason 本身不代替终止符 | tool_calls 按 index 拼接 name/arguments；尾部 usage；显式 stream_options HTTP 400 才去掉重试一次 |
| Anthropic | SSE type/event；message_stop 终止 | content_block_start + input_json_delta.partial_json；message_start 输入、message_delta 输出用量 |
| Gemini | streamGenerateContent?alt=sse 的 data 候选；finishReason 终止 | 完整 functionCall parts 与 thoughtSignature 回放；usageMetadata，独立于正文 |
| Ollama | NDJSON；done:true 终止 | message.content 增量、完整 tool_calls；prompt_eval_count/eval_count |

共同点：输入字节缓存到完整行/事件，不能按字符串切 chunk；usage 缺失仍未知；连接 EOF/断流/超时不静默成功。显式 SSE id 才能识别重传；没有 id 的相同 token 是合法文本，不能猜作重复。前端另用应用事件 seq 去重，不把 SSE id 或持久消息 seq 混作事件 seq。

## 未验证 / 偏离

- **真实 BYOK、手机实机明确排除**：OpenAI 兼容 zai/glm-5.3-flash 后续由父会话实测；其余三家不能以 mock 冒称真实端点通过。Android IME/旋转/读屏器、Windows/macOS、原生 Wayland、桌面真实键盘操作均未验收。
- OpenAI 参数兼容重试采用保守的「400 明确提及 stream_options」分支，而非任意 400；避免一般坏请求自动重发付费链路。
- 工具执行过程仍为原有离散 progress；不输出思考链，工具过程/能力/降级徽章 UI 与引用遗留不在本批顺带实现。
- Xvfb 应用日志仍有既有 `menu render selftest FAILED: checked 条目未打勾`，本批没有改菜单；不作为流式验收通过项。
- Reviewer gate 由父会话在 push 后执行，有问题追加修复提交，不将本记录等同整体阶段①/② ship。
