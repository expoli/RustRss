# MCP 集成

RustRss 提供 MCP server，让支持 MCP 的客户端读取本地订阅和文章。桌面应用设置页可以启用服务、管理 token，并复制客户端配置。

## 安全默认值

- HTTP 服务只绑定回环地址，默认端口为 `8817`。
- `/mcp` 请求必须带有效 token；token 可在“设置 → 外部集成 → MCP”查看或轮换。
- `/health` 不要求 token，只报告服务是否存活，不返回订阅数据。
- **默认只读**。写能力需显式生成写 token 并开启写开关；退订等危险操作还有独立开关和确认参数。

客户端配置中的 token 等同于访问凭据，请不要提交到代码仓库或公开分享。完整数据与安全说明见[隐私与数据](privacy.md#mcp-访问)。

## 连接客户端

设置页可复制 JSON 配置或 Claude Code 命令。HTTP 客户端配置形如：

```json
{
  "mcpServers": {
    "rustrss": {
      "type": "http",
      "url": "http://127.0.0.1:8817/mcp",
      "headers": {
        "Authorization": "Bearer <从 RustRss 复制的 token>"
      }
    }
  }
}
```

客户端还必须接受 `application/json, text/event-stream`。

## 工具范围

只读工具覆盖订阅源、文件夹、文章列表、单篇正文、搜索、未读统计、标签和主题信息。列表默认每次最多返回 10 篇，最多 50 篇；正文通过 `get_article` 单独读取。

启用写能力后，客户端还可按授权范围管理阅读状态、刷新订阅、补全文、管理订阅/文件夹/标签和调整主题。详情以 MCP 工具说明为准。桌面应用与独立 `rustrss-mcp` 使用同一份 SQLite 数据库。

## 独立服务器

不启动桌面窗口时，可以运行 MCP 二进制：

```bash
cargo run -p rustrss-mcp -- --http 127.0.0.1:8817
```

也可以省略 `--http` 以 stdio 方式交给 MCP 客户端启动。HTTP 服务启动时会输出 token；请只在本机受控环境中使用，并按客户端配置要求保管。
