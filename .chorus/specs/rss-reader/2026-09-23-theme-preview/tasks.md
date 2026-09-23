# 分阶段任务与依赖

状态：本地任务草案；尚未物化为 Chorus 实现任务，未开始产品改动。

| 任务 | 依赖 | 交付与验收 | 主要范围 |
| --- | --- | --- | --- |
| T0 设计与底层截图探针 | 无 | PRD/设计/任务/交互稿落库；Linux native snapshot 变色连续 6 帧证据；三平台官方 API 路径；局限列出 | 本目录 |
| T1 Tauri 截图集成 spike | T0 | 同一 Tauri 窗口新帧/缩放/隐藏/超时验证；PNG 输出有界；Win/mac API 编译及各平台运行证据分开报告 | 隔离 example/feature、平台适配器 |
| T2 core 主题模型与存储 | T0 | 三预设明暗配色、patch/schema/旧设置迁移、CAS/同值短路/有限历史；并发写与恢复验证 | core/theme、store、测试 |
| T3 主题渲染与真实 fixture | T1,T2 | 统一 token/共享组件；总览/文章/设置场景；生产热路径不全量重建；语义颜色清单 | ui、i18n、桌面胶水 |
| T4 外观页与设置重组 | T2,T3 | 七类设置、Aa 共用字段、操作归位；预览/恢复、键盘导航与双语；旧功能均可达 | ui、commands |
| T5 MCP 配置读写 | T2 | 5 个 get/list/validate/update/restore 工具；与 UI 等效；默认权限/轮换/审计/离线能力；更新同步 | mcp、core、桌面事件 |
| T6 MCP 截图预览闭环 | T1,T3,T5 | 3 个 preview/capture/finish 工具；会话/CAS/版本握手；内嵌与 stdio 桥接；图片+元数据；超时/撤销/崩溃回收 | mcp、桌面适配器、ui |
| T7 聚合验收与文档 | T4,T6 | 全量回归、真实客户端图片反馈闭环、跨平台矩阵、更新 README/spec/验证清单；未测项不勾 | 测试、文档 |

## 执行顺序与退出条件

先做 T1，尽早验证最大不确定性；T2 随后逐项实施，按 T3→T4→T5→T6→T7 串行推进。现有工作区已有其它修复，不在本批次批量提交或覆盖。

T1 若仅部分平台跑通，记录能力矩阵并实现明确的 unavailable；不能据此勾选三平台验收。若截图路径失败，继续完善配置模型与 UI，但 MCP 图片闭环保持未完成。不得静默换成 Chromium 或桌面截屏。

后续正式提案按 chorus-feature-pipeline 进行 proposal/task/aggregate 评审，创建任务时将上表验收拆成可单独核验的 AC。本轮只完成设计与可行性探针，不执行 worker 的 commit/push/发布流程。

## 回归重点

- core：旧配置/坏配置/未知 schema、部分更新、CAS 竞争、历史恢复、同值零写；所有读取与写入共用 core。
- MCP：read/write gating、token 轮换、write_enabled 切换、跨 profile/预览访问、内嵌与独立部署、图片消息结构与体积。
- UI：三预设 × 两种模式 × 三场景；中英文/缺字体/长标题/空数据/错误；125/150/200% 缩放与窄窗；列表选择与正文锚点。
- 预览：字体延迟、陈旧 ready、迟到回调、并发 patch、deadline、取消/重试保存、用户期间手动改主题、应用退出。
- 性能：颜色修改 DOM 写次数、revision 轮询查询与开销、截图线程阻塞、临时窗口/PNG 回收；不将 Xvfb 热态截图数字用作冷启动承诺。

## 本轮状态

- [x] 已形成设计、任务草案与独立交互稿。
- [x] Linux WebKitGTK 原生截图探针：三配色循环两轮成功，详见证据。
- [ ] Tauri 原生适配器与 MCP 集成。
- [ ] Windows/macOS/Wayland 运行验证。
- [ ] 正式提案评审、任务物化与产品实现。
