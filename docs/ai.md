# 内置 AI

RustRss 可把文章发送到你配置的模型服务，生成摘要或翻译。AI 是可选功能，使用前请先在“设置 → AI”选择服务商、模型并测试连接。

支持的服务：

- Ollama
- OpenAI 兼容 API
- Anthropic API
- Gemini API

不同服务商可配置自定义端点。使用本地 Ollama 时，请确认本机服务已启动且模型可用。

## API 密钥

API 密钥存入操作系统凭据库：Linux Secret Service、macOS Keychain 或 Windows 凭据管理器。界面只显示密钥是否已设置，不会把密钥写入 SQLite 配置。Linux 凭据库不可用时，应用会提示原因；也可临时使用 `RUSTSS_AI_KEY` 环境变量。

## 使用摘要与翻译

打开文章后，选择“AI 摘要”或“AI 翻译”。结果会标明是否来自缓存；需要新结果时可以重新生成。超长正文会按上限截断并标记。

AI 请求会把文章内容发送到所选服务商。使用在线服务前，请确认服务商的数据处理政策符合你的需要。详见[隐私与数据](privacy.md#ai-请求)。
