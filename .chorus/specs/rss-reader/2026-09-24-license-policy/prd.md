---
title: PRD: 许可协议切换为 AGPL-3.0-or-later + 商业授权双轨
proposalUuid:
documentUuid:
---

# PRD: 许可协议切换为 AGPL-3.0-or-later + 商业授权双轨

> 本文件夹为本地变更记录（spec-lite，`CHORUS_SPEC_MODE=lite`）；`proposalUuid` / `documentUuid` 待本仓建立
> Chorus 提案后回填并镜像为 Document。

## 背景

仓库自脚手架起（`2026-09-20`）按 `MIT OR Apache-2.0` 双许可分发（`Cargo.toml` 的 workspace `license`、
`README.md` 许可证段、`ui/index.html` 关于面板硬编码显示）。该口径是 Rust 生态惯例，但**允许任何人闭源商用**。

`.chorus/specs/rss-reader/2026-09-20-initial-requirements/prd.md:154` 早已把许可证列为待用户决策项：
folo 为 AGPL-3.0、MrRSS/Papr 为 GPL 系。用户本次明确目标：**自己可商用；别人要闭源商用须另行获准；
别人可 fork、可修改；修改后仍须递归开源**。

## 决策（2026-09-24 用户拍板）

采用 **AGPL-3.0-or-later 开源授权 + 独立商业授权** 的双轨模式（经典 dual licensing）：

- 默认授权改为 `AGPL-3.0-or-later`（SPDX），仍是 OSI 开源许可，发行版与社区不受阻；
- 不愿承担 AGPL 源码开放义务（闭源分发、网络服务不公开源码、并入闭源产品、合规排除 copyleft）的
  商业使用者，须另行取得书面商业授权；
- 配套商标政策与贡献授权条款，保证双轨可执行。

## 目标与验收

- [ ] 仓库根 `LICENSE` 为 AGPL-3.0 官方全文（未改一字，sha256 校验）
- [ ] `Cargo.toml` 的 `license = "AGPL-3.0-or-later"`，三个 crate 通过 `license.workspace = true` 继承
- [ ] `README.md` 许可证段说明默认授权、商用何时须获准、历史版本边界
- [ ] 应用「关于」面板显示 `AGPL-3.0-or-later`（`ui/index.html`）
- [ ] 商业授权入口（`LICENSE-COMMERCIAL.md`）、商标政策（`TRADEMARK.md`）、贡献授权条款（`CONTRIBUTING.md`）齐备
- [ ] 历史 MIT / Apache 文本保留在 `LICENSES/` 并注明适用边界提交
- [ ] 依赖许可与 AGPL 兼容（无 GPL/AGPL 不兼容依赖），可复核
- [ ] 回填 `2026-09-20-initial-requirements/prd.md` §9 待决策项 2
- [ ] `cargo test --workspace` 全绿，无回归

## 非目标

- 不追溯撤销既有授权：切换前发布的版本与构建产物仍按 `MIT OR Apache-2.0`，不可回收。
- 不引入 per-file SPDX 头（240 个提交、数百文件，收益低于噪声）。
- 不作为法律意见；商业授权合同与正式 CLA 交由律师起草。
- 不改变任何功能行为、数据路径与 MCP 安全口径。

## 影响面

| 文件 | 变更 |
|---|---|
| `LICENSE` | 新增：AGPL-3.0 官方全文 |
| `LICENSES/{LICENSE-MIT,LICENSE-APACHE,README.md}` | 旧授权文本移入 + 边界说明 |
| `LICENSE-COMMERCIAL.md` / `TRADEMARK.md` / `CONTRIBUTING.md` | 新增 |
| `Cargo.toml:8` | `license = "AGPL-3.0-or-later"` |
| `README.md` 许可证段 | 重写 |
| `ui/index.html` 关于面板 | 许可证字符串 |
| `2026-09-20-initial-requirements/prd.md:154` | 回填决策 |
