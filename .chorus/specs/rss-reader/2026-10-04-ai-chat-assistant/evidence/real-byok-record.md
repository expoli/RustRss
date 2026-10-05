# 真实 BYOK 实测记录（2026-10-04）

## 环境
- 桌面 debug 构建（阶段③完成 `3e5e1a4` 后），Xvfb :99，隔离实例
- 端点：智谱 OpenAI 兼容 `https://open.bigmodel.cn/api/paas/v4`
- 模型：`glm-4-flash`（免费档；`glm-5.3-flash` 该账户余额不足 1113 未验）
- key 经 `RUSTSS_AI_KEY` 环境变量后备通道注入（不落库/不落日志）

## 结果：OpenAI 兼容 provider 闭环 ✓
- 端点探测：非流式 ✓ / 流式 SSE ✓（中文多字节分块）/ tools ✓（finish_reason=tool_calls，
  标准 OpenAI tool_calls 形态、args 字符串）
- 应用内闭环（截图 /tmp/rss-audit/c*.png）：
  1. 侧栏「AI 助手」→ 聊天视图（模型行 openai_compatible · glm-4-flash）✓
  2. 键入「搜索 rust 相关的文章，打开第一篇，用中文总结三个要点」→ 发送
  3. **隐私确认对话框**（端点/会话范围「未绑定日报（无资料）」/可能发送日报正文/
     按 token 计费/停止不保证撤销费用/本地删除不清除服务商记录）→ 同意并发送 ✓
  4. AI 回复：《Announcing Rust 1.98.1》三要点总结——内容与该文实际发布公告吻合，
     证明工具循环 search_articles → get_article → 回答真实执行 ✓
  5. 数据库回读：chat_sessions 1 行（title 自动生成/provider/model/endpoint_id 哈希）；
     chat_messages seq1 user done + seq2 assistant done，
     **input_tokens=5870 / output_tokens=130**（工具结果回灌的痕迹，纯单轮不可能）；
     bodies 含用户消息与助手总结全文 ✓

## 四家实测状态（设计要求的分别记录）
| Provider | 状态 |
|---|---|
| OpenAI 兼容（智谱 glm-4-flash） | **闭环已验**（含流式 SSE、tools、隐私确认、持久化+usage） |
| Anthropic | 未验（无 key） |
| Gemini 原生 | 未验（无 key；签名回放有协议夹具覆盖） |
| Ollama | 未验（本机未安装；降级路径有协议层覆盖） |

## 待真机/后续
- 手机实机（320px/IME/旋转）、Anthropic/Gemini/Ollama 真实 key 到位后补验

## 附：并发提取真实端点测量（2026-10-05，第一批优化）
- 端点/模型：同上（OpenAI 兼容 · glm-4-flash 免费档）
- 场景：24 篇测试文章，DigestExtractionScheduler 并发上限 4（渐增），真实 HTTP
- 结果：**24/24 成功，耗时 19.5s**（≈0.81s/篇含限流自适应）
- 对照：串行实现同规模估算 36-48s → 约 2-2.5×（免费档限流约束下；付费档并发上限更高）
- 运行方式：`RUSTSS_AI_KEY=… cargo test -p rustrss-core --test ai_chat_real_endpoint -- --ignored --nocapture`
  （tests/ai_chat_real_endpoint.rs，ignored 测量，CI 不跑）

## 测试 APK（2026-10-06）
- /tmp/rss-audit/rustrss-0.6.0-mobile-v11-release.apk（aarch64 39.3MB，含并发优化 + 全部修复，发布签名）
