# MCP 预览改为本地截图文件（2026-09-24）

本次为用户明确要求的返回契约变更。preview_theme 与 capture_theme_preview 不再返回 MCP ImageContent/base64，仅文本/structuredContent，包含绝对 image_path、image_mime_type、image_bytes、image_expires_at_ms、image_read_instruction，保留逻辑/像素尺寸、版本、hash、场景等证据。

## 客户端流程

1. get_theme 查看 capabilities.preview.image_delivery=local_file 与 requires_shared_filesystem=true。
2. preview_theme 返回路径；使用 view_image 或等价工具实际打开图片。
3. 调整后 capture_theme_preview 返回新的独立路径，再读图。
4. 选择 save/cancel；已产生的图保持可读，直到生成后600秒到期（5秒清理周期）。

无本地读图能力、远程机器或容器无法访问路径时，应明确报告视觉验证不可用。工具描述直接解释这一点。旧客户端应刷新工具目录。不是把路径/尺寸当成模型看过图片。

## 文件规则与实现

- core/theme_preview_files：随机私有目录；Unix 0700/0600；独立、不可覆盖的文件名；完整写入/关闭后才公布路径。无客户端自定义输出路径。
- 单张2MiB、每服务32张/32MiB，写前检查；满额保留旧图并返回 preview_file_limit，提示等待到期。I/O失败返回 preview_file_unavailable，保留候选ID便于取消。
- token失效时删除相关图；保存/取消不删除；桌面 RunEvent::Exit 与 MCP stop 主动清理并禁止晚到渲染写文件。独立 stdio/HTTP 桥接透传桌面路径。
- 正常到期以单调时钟判断，对外附墙钟毫秒期限。失败删除保留配额记账、下次重试。仅管理本实例已登记路径。
- 强杀/崩溃可能残留本次目录，需要 OS 临时目录维护；不扫描其它实例，也不承诺进程已死后仍有后台计时器。

## 验收

- 新 MCP 文件契约测试先红后绿；目录权限测试也抓到 tempfile 默认目录权限不足，改为创建时显式0700后通过。
- core覆盖唯一文件、字节回读、到期边界、失权、Drop清理、数量/字节闸门、Unix权限。
- MCP覆盖无内联图、能力与工具描述、保存/取消可读、重拍独立路径、背景失权清理、桥接读文件、容量失败与候选可取消、显式shutdown删除目录与阻止晚写。
- cargo test --workspace：413 passed / 0 failed；Node：40 passed；clippy仅原有3条core告警；cargo build --workspace成功。
- 新产物原生 KDE Wayland：verify-theme-preview.py --display wayland，24图均从路径读取并检查PNG/像素尺寸；同库独立stdio桥接、临时不写库、幂等保存和失权回收通过。本轮不重新验证原生点击，本脚本 local_cancel=false。

机器结果见 [文件输出证据](mcp-preview-files-results.json)。未重复 Xvfb 双档矩阵或30分钟长测；该次文件600秒TTL覆盖来自单调时钟边界测试；后续已以无模型脚本补验真实600秒回收，见 [补验报告](file-probe-ui-followup.md)。Windows/macOS仍未运行验收。

## 实际读图与正常退出

另起隔离原生Wayland桌面，通过正式HTTP MCP创建/重拍两张PNG，各1920×1280、约317KB；权限检查0700/0600，取消后两文件仍可读，直接SQLite回读无正式主题写入。随后通过应用exit_app命令正常退出，确认进程结束且截图目录整体消失。

非交互Codex会话在首次get_theme被其审批策略拒绝（MCP tool call requires approval, but approval policy is never），没有将其计为自动闭环通过，也未修改其审批策略。主助手按用户授权执行MCP后，让现有交互Codex只用view_image读取返回文件：第一图完整；第二图又被模型描述为色条。两份原始PNG正文逐像素相同，仅64×8新帧标记不同。

**文件契约与存储验收通过，但文件输出没有保证消除模型连续读近似图的异常。** 本轮不能宣称完整客户端视觉闭环无异常；下一步应定位客户端请求/服务端图像处理与模型解释层，不能用改变截图内容来掩盖未确定根因。

## 前置专项定位证据（只读，未修改客户端）

Pi/deepseek-flash在用户授权的Herdr pane协助查阅精确版本rust-v0.156.1源码，主助手核对相关路径：MCP image转换为完整data URL，图片预处理按预算整幅缩放或原字节传递，所查路径未发现近似图差分裁剪。不能据局部源码排除整个客户端或服务端。

- [MCP内容转换](https://github.com/openai/codex/blob/rust-v0.156.1/codex-rs/protocol/src/models.rs)
- [图片准备](https://github.com/openai/codex/blob/rust-v0.156.1/codex-rs/core/src/image_preparation.rs)
- [整图尺寸预算及字节缓存](https://github.com/openai/codex/blob/rust-v0.156.1/codex-rs/utils/image/src/lib.rs)

对先前异常PNG做未改像素的对照：单张作为新会话附件→完整；两张同时作为新会话附件→均完整；旧会话只读一次文件→完整；本次连续读两张文件→第二张色条。对照改变了入口/上下文，未锁定单一因果，也没有采集真实上行模型请求。原计划本地请求接收器实验尚未执行，不算已验证。
