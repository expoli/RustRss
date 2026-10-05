# 桌面产物重建复测（reviewer note：追加 JS 修复的产物验证，2026-10-04）

产物：debug 构建（嵌入 HEAD 含 B′ 全部 UI 修复），测试库（新增 IT之家→「工具」源标签）。

| 场景 | 结果 | 证据 |
|---|---|---|
| a. 列表条目 chips 继承源标签 | IT之家 215 篇文章全部显示「AI」chip | rt1-list-zoom.png |
| b. 阅读页操作行继承 chip | 「更多 [AI] +标签」中 AI chip 可见 | rt3-picker-zoom.png |
| c. 侧栏标签计数含继承文章 | 「AI」计数 0 → **204**（IT之家 215 篇中未读口径） | rt4-tags-zoom.png |
| d. 打标选择器只读语义 | vm 层测试 10/10（tags.test.cjs 含来源/只读断言） | scripts/tests/tags.test.cjs |
| e. 聊天视图回归 | 打开正常（无回归症状） | real13 启动后操作 |

结论：追加 JS 修复在重建产物上全部生效。
