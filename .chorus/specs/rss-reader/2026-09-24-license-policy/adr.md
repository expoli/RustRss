---
title: ADR: 许可协议切换为 AGPL-3.0-or-later + 商业授权双轨
proposalUuid:
documentUuid:
---

# ADR: 许可协议切换为 AGPL-3.0-or-later + 商业授权双轨

- 状态：已接受（2026-09-24 用户拍板）
- 边界提交：切换前 HEAD = `cfaf0dc6be8e6fb1f279bb06204ca80d76d81f18`（`nightly` 标签指向 `79d65ec`）

## 上下文

需求原话：自己可商用；别人商用须另行获准；别人可 fork、可修改；修改后不能商用（可自用）。

"非商业许可"路线（PolyForm Noncommercial 1.0.0 等）能字面满足该原话，但代价是**不再是开源**：
违反 OSD 第 6 条（不得歧视领域），发行版不收录，社区口碑受损。

用户权衡后选择：以「事实上的闭源商用不可行」替代「字面上的商用须获准」，换取保留开源属性。

## 决策

**AGPL-3.0-or-later（开源）+ 商业授权（闭源商用）双轨**，接受一处措辞上的偏离：

> **法律口径与原始需求有一处偏差**：AGPL 下别人的商用是被允许的，代价是必须递归开源其修改；
> 闭源商用事实上不可行，想闭源就得上門购买授权。

## 备选方案与剔除理由

| 方案 | 剔除/采纳理由 |
|---|---|
| **保持 MIT OR Apache-2.0** | 与目标完全相反：任何人可闭源商用 |
| **PolyForm Noncommercial 1.0.0**（+商业授权） | 字面满足原话（非商业用途授权、Changes/New Works 许可允许修改、商业须另行获准），但不再是开源，发行版不收录，GitHub licensee 大概率识别为 Unknown。**作为未采纳的备选记录在此** |
| **BSL 1.1**（MariaDB/HashiCorp 系） | 强制 Change Date ≤ 4 年且到期自动转 GPL 兼容开源协议，无法"永久须获准"；且允许非生产环境自由商用，比目标更松 |
| **FSL-1.1**（Sentry 系） | 只禁竞品用途，非竞品商用允许；且 2 年后自动转 MIT/Apache |
| **Elastic License 2.0** | 允许商用，只禁作为托管服务对外提供 |
| **Prosperity Public License 3.0.0** | 条款最近，但内置 30 天免费商用试用，与"商用须先行获准"不符 |
| **CC BY-NC-4.0** | ❌ 不适合软件：无专利授权、无源码/分发语义，"非商业"定义更含糊（Creative Commons 官方亦不建议用于软件） |
| **自写非商业 EULA** | ❌ 无 SPDX ID → Cargo/工具链报错；可读性与法律稳健性差 |
| **AGPL-3.0 + 商业授权** | ✅ 采纳 |

## 后果

### 正面

- 仍是 OSI 开源：发行版收录、社区口碑、可上 crates.io（`AGPL-3.0-or-later` 是合法 SPDX 表达式，且属
  choosealicense 语料，GitHub 许可识别比 PolyForm 可靠）。
- 闭源厂商只剩两条路：合规开源其修改，或购买商业授权。竞品因此无法把本项目做成闭源商业产品。
- AGPL §13 与 MCP 的网络形态天然契合：把 `rustrss-mcp` 当托管服务对外提供者，必须提供 Corresponding Source。
- 含专利授权；违规须书面通知后 30 日内整改，动作链清晰。

### 负面（如实记录）

- **不追溯**：`cfaf0dc` 及更早发布版本、既有 fork、切换前 `nightly` 安装包仍按 MIT OR Apache-2.0，
  可继续闭源商用，无法回收。`nightly.yml` 每次"旧资源先删后传"，下一次 master push 后切换前的
  安装包资产即被替换为新授权产物。
- **Apple 生态上架受限**：AGPL 与 Apple App Store 条款冲突，本项目不上 Mac App Store（直发 dmg 不受影响）。
- **企业采用门槛提高**：排除 copyleft 的公司内部政策会直接卡住；这也是商业授权存在的意义。
- **商业授权能力取决于版权集中度**：目前 240 个提交全部来自单一版权人 `expoli`（`git log --format=%ae` 全量核对），
  切换与双授权干净可行；一旦接受外部贡献而未签约，将永久失去对其贡献部分的商业授权能力 →
  已在 `CONTRIBUTING.md` 内置贡献授权条款（inbound=outbound + 再许可权）。

## 依赖兼容性复核（2026-09-24）

`cargo metadata` 全量 650 个包，许可分布：MIT / Apache-2.0 系为主，8 个 MPL-2.0
（`cssparser` / `selectors` / `dtoa-short` / `option-ext` / `cssparser-macros`，MPL §3.3 允许并入
(A)GPL Larger Work），`BSL-1.0` 实为 **Boost Software License**（`clipboard-win` / `error-code` / `ryu`，宽松），
`r-efi` 为 `MIT OR Apache-2.0 OR LGPL-2.1-or-later` 可选项（取 MIT/Apache）。
**结论：无 GPL/AGPL 不兼容依赖，切换合法。** 系统库 WebKitGTK（LGPL，动态链接）不受影响。

后续闸门：禁止引入 GPL/AGPL/SSPL-only 之外的**不兼容**依赖；新增 copyleft 依赖前先复核。

## 执行清单（本次落地）

见同目录 `prd.md` 的「影响面」表；已完成项在验证记录中勾选。

## 后续动作

1. 回填 `2026-09-20-initial-requirements/prd.md` §9 决策 2（本次同步完成）。
2. 用户补充**商务联系邮箱**替换 `LICENSE-COMMERCIAL.md` / `TRADEMARK.md` 中的 GitHub 链接。
3. 正式商业授权合同模板与正式 CLA：交律师起草（未做，非阻塞）。
4. 本文件夹的 `prd.md` / `adr.md` 待建 Chorus 提案后补 `proposalUuid` / `documentUuid` 并镜像 Document。
