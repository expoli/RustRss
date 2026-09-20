---
title: PRD: 系统托盘（显示/隐藏 + 退出，含不可用降级）
proposalUuid: e64bf55d-f33b-4cee-a271-9af495b6b68f
documentUuid: f6c397b5-a3e4-48f0-983d-1107854731f7
---

# 系统托盘 — 需求与取舍

> 状态：待评审。需求基线见 `.chorus/specs/rss-reader/spec.md`「托盘图标……任一能力不可用时显式降级而非崩溃」（本文件只承载分析与取舍，不重复验收条目）。

## 1. 背景与问题

spec 要求托盘在 X11/Wayland 下行为一致且不可用时显式降级；当前 src-tauri 无任何托盘代码，是「Linux 双会话行为一致」验收组里唯一完全缺失的能力。

## 2. 已定决策（elaboration round 1，2026-09-20）

| 决策点 | 选择 | 理由 |
| --- | --- | --- |
| 功能集 | 最小集：显示/隐藏窗口 + 退出 | spec 只要求托盘行为一致与降级；未读数/快捷操作是后续增强 |
| 关闭按钮行为 | v1 保持直接退出，不改 | 行为变更超出「补托盘」范围 |
| 不可用降级 | 托盘构建/初始化失败则跳过（日志说明），主流程完全可用 | spec 明文「显式降级而非崩溃」 |
| 验收口径 | headless 环境验代码路径 + 可单测部分；真实托盘行为诚实标注「需桌面环境人工验证」 | 客观环境限制，不谎报完成（先例：T2 real-site 披露） |
| 文档义务 | README 特性清单 + spec.md 托盘小项如实回填（注明验证范围） | AGENTS.md 义务 |
| 技术选型 | Tauri 2 core 的 TrayIconBuilder（不引入额外插件，除非实现者确认必需） | 最小依赖；图标复用 src-tauri/icons/ |

## 3. 改动面

| 文件 | 改动 |
| --- | --- |
| `src-tauri/src/main.rs` | 托盘构建（setup 阶段），失败降级路径 + 日志 |
| `src-tauri/tauri.conf.json` | tray 相关声明（如需） |
| `src-tauri/Cargo.toml` | 仅当 core tray 能力需要 feature 时调整 |
| `README.md` / `spec.md` | 特性与验收点如实回填 |

## 4. 非目标

- 不做未读角标/气泡。
- 不做托盘菜单内的订阅操作。
- 不改窗口关闭行为（v1 直接退出）。

## 5. 风险

- Wayland（尤其无 XWayland）下 StatusNotifierItem 不可用：降级路径必须先于功能验证，防「起不来」。
- tray 事件循环与 Tauri 单实例窗口管理的交互：显示/隐藏要处理窗口已销毁/未创建的边界。

## 6. 评审记录

- Round 1 待审。
