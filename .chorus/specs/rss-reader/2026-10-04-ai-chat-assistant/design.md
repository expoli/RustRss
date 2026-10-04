# 应用内 AI 对话助手 · 立项设计（oracle 评审定稿 2026-10-04）

状态：已批准立项，按阶段实施。继承决策：BYOK；手机/桌面同优先；只读工具先行；
日报保留；不引入外部 MCP 客户端生态。

## 架构
- core 新增 `ai/chat`（协议类型、provider adapter、agent 循环、预算/取消）+ `store/chat`（持久化）；Tauri 仅凭据快照/启动停止/事件转发。
- 保留 `AiRequest` 兼容摘要/日报；新增 `ChatRequest{system,messages,tools,limits}`；消息块 `Text / ToolCall{id,name,args} / ToolResult{call_id,data,error}`；响应含 blocks、结束原因、usage；保留 provider 回放字段（Gemini 签名）。
- **现在定义流式事件接口**（非流式也走 started/progress/done/error 生命周期）；SSE/NDJSON 后置。
- 工具：MCP 只读查询/投影**下沉 core 共享工具门面**（MCP 包装器继续调用）；聊天静态正向白名单 10 项（订阅/分组/文章列表/搜索/正文/未读统计/库统计/标签/日报列表/日报正文）。不开放主题/刷新/抓全文/生成日报/SQL/URL。简单 JSON Schema 子集 + Rust 参数校验；普通文本绝不解析为伪工具调用。工具执行后端强制注入会话范围；查询短持锁、网络锁外。
- 修正事实：`RustRssMcp` 持有自己的 Store/锁/Fetcher——core 反向依赖会循环；TOOL_SPECS 无参数 schema/执行器；`get_article_json` 正文无截断、list_feeds/folders 无分页——MCP 口径不能替代聊天预算；schema 基线 1/迁移后 2，聊天走追加迁移。

## 模型兼容与诚实降级
- Ollama 聊天走 `/api/chat`（`/api/generate` 不能承接工具消息）。
- 能力状态按 provider+端点+模型缓存（未知/已确认/不支持）；显式能力测试用合成数据并提示可能计费；连接测试成功 ≠ tools 能力。
- 仅明确「不支持 tools」错误才降级；401/429/超时不是能力不足。
- 降级 = 本地 FTS 检索注入：可编辑关键词+范围，≤10 条短摘要 + 3 段相关正文 + 选中日报相关章节；证据 ≤8,000 字符再受上下文预算裁剪；界面明示「本地检索辅助模式」与截断；找不到证据就请缩小范围，不凑答案。

## 预算/取消/历史（初始护栏，试验后定标）
- 每用户回合 ≤6 次模型请求、≤10 次工具调用、≤120s；重复相同查询无新信息即终止。
- 输入 ~12k tokens、输出 ≤4k；回合累计 ~32k、会话 ~200k 后要求明确继续。
- 工具结果硬字节闸：单结果 12KiB、回合证据 48KiB；长文片段读取（获取时限额）。
- 记录 provider usage（缺失标未知，不按字符数冒充账单）。
- 历史裁剪按完整回合（工具调用与结果不可拆散），告知用户，不自动付费总结。
- 会话单 flight + Drop guard；停止取消在途请求；已发生费用不保证撤销；重试不自动重发已付费链路。

## UI/UX
- 日报目的地内部加「日报 / AI 助手」分段；日报详情「讨论这份日报」（绑定真实范围+版本）；无日报也能进助手。桌面侧栏加助手入口；手机独立聊天容器/路由（不靠 .reader-head MutationObserver 猜导航）。
- 最小界面：模型与范围、消息、输入框、发送/停止、新会话/历史、来源引用、错误重试；工具过程可见（「正在搜索文章→读取3篇→整理」），不展示思考链。
- 首次发送确认端点/会话范围/可能发送正文；扩大范围或换端点重新授权。
- 输入框随 IME 避让；中文 composition 期间 Enter 不发送；返回先关键盘/引用页再回原位；离开不取消任务、完成不抢导航。
- 流式仅增量更新当前消息；用户上滚不强制滚底；未配 key 给设置入口；限流/超预算/网络错误保留草稿与已收内容。

## schema（元数据与大正文分表）
- chat_sessions(id,title,provider,model,endpoint_id,scope_json,created_at,updated_at)；索引 (updated_at DESC,id)。
- chat_messages(id,session_id,seq,run_id,role,status,input_tokens,output_tokens,created_at)；FK 级联；UNIQUE(session_id,seq)。
- chat_message_bodies(message_id,parts_json)；按当前消息页读取。
- chat_sources(message_id,source_key,kind,source_identity,revision,hash,title,excerpt)；摘录有界，不随源文章删除级联消失。
- 启动遗留 running 标记 interrupted，不自动续费重试；不按年龄偷偷删除（手动清空+可选保留期限+配额提示）；会话不存 key；SQLite 聊天内容非加密保险箱，整库备份含它们。

## 阶段与硬验收
| 阶段 | 交付与验收 |
|---|---|
| 0 契约与试验 | 修日报页状态条（已修）；同步隐私/spec；四 provider 请求/响应夹具与能力试验后再估工期 |
| ① 可追问的日报会话 | 双端聊天+持久化+取消+引用；非流式无自主工具；绑定「这份报告」；验证多轮/重启/中断/换端点授权 |
| ② 只读资料助手 | 共享工具门面+四 provider 工具适配+诚实降级；越权/未知工具拒绝、阅读状态不变、预算/重复熔断、标签与 MCP 同源 |
| ③ 流式与体验 | SSE/NDJSON 分包、UTF-8 边界、断流、重复/迟到事件；长会话分页、滚动、无障碍 |

真产物验收：手机 320px/大字号/横竖屏/键盘/后台恢复 + 桌面键盘操作；**真实 BYOK 检索→取文→回答闭环必须实测**（mock/CI 不替代），四家实测状态分别记录。

## 风险
- RSS 正文与模型 Markdown 都是不可信输入：防 prompt injection 靠后端范围/白名单；渲染禁脚本/自动远程图/危险链接；订阅 URL 凭据不进 prompt。
- 引用由应用校验生成（稳定实例身份/日报版本）；退订后聊天引用片段的同步删除确认与隐私说明；清本地会话 ≠ 删除服务商记录或旧备份。
- Android localhost 是手机本体；连电脑 Ollama 需明确地址提示。
- 待跟踪欠项：日报范围错配、返回栈、标签 B′——不被新立项掩盖；工期估算不作交付承诺。

## 附：日报页布局修复（本批已修）
digest 页 .right-col 改 block 后 .statusbar 跟随内容形成中部状态条 + 底部 #m-status 镜像双显；
.reader 继承阅读页顶部 inset/58px 返回槽。修复：digest 页隐藏 .statusbar、.right-col 恢复
flex column、#reader 覆盖阅读页 padding（digest-home 标记切换）。
