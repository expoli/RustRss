---
name: chorus-feature-pipeline
description: 用 Chorus 把需求从 idea 推到 ship 的可复制流程与实操要点：四道闸门（proposal/task/aggregate 评审）、mcpScript 的返回包封解包、chorus CLI 的多 agent 选择与文档镜像、single-writer 串行纪律、scope 例外与 FAIL 处理、durable spec 勾选纪律。当要在 Chorus 管理的仓库里走「idea → 提案 → 任务 → 评审 → 收口」流程时使用。
---

# Chorus 特性流水线实操手册

## 标准链路

```
checkin → chorus_pm_create_idea → chorus_claim_idea
  → chorus_pm_start_elaboration（真实决策点，配 AskUserQuestion）
  → chorus_answer_elaboration → 评论确认 → chorus_pm_validate_elaboration（resolve）
  → spec-lite 文档（.chorus/specs/<slug>/<date>-<change>/prd.md + tech_design.md）
  → chorus_pm_create_proposal（description 带 locator 行）
  → 文档镜像（--arg-file）→ chorus_pm_add_task_draft × N（带依赖）
  → chorus_pm_validate_proposal → chorus_pm_submit_proposal
  → 【闸门 1】chorus-proposal-reviewer → 按 VERDICT 修 → chorus_admin_approve_proposal（物化任务）
  → 每任务：chorus-worker（claim→实现→测试→commit→push→report→自检→submit_for_verify）
  → 【闸门 2】chorus-task-reviewer → chorus_mark_acceptance_criteria → chorus_admin_verify_task
  → 【闸门 3】最后一个任务后：chorus-code-reviewer（idea 聚合复审）
  → chorus_create_report（完成报告）→ 收口（README/spec 勾选/验证清单）
```

## 工具调用要点（踩过的坑）

- **mcpScript 里的返回是 MCP 包封**：`r.data` 形如 `{content:[{type:"text",text:"<json>"}]}`，必须 `JSON.parse(content[0].text)` 才能拿到字段（直接用 `mcp` 工具调用则看到平铺 JSON，两种形态都要能处理）。
- **`chorus` CLI 需要指定 agent**：本机有多个 label（`chorus-dev`/`x7-dev`/`pi-dev`）；RustRss 用 `--agent pi-dev`。
- **文档镜像**：`chorus mcp call --agent pi-dev chorus_pm_add_document_draft '{"proposalUuid":"…","type":"prd","title":"…"}' --arg-file content=<file>`；已物化的改 `chorus_pm_update_document '{"documentUuid":"…"}'`。**别把正文重打进参数**（烧 token 且会漂移）。
- **pending 状态不可改草案**：reviewer FAIL → 先 `chorus_pm_reject_proposal` 回 draft → 改 → 重提。
- **物化后改任务**：`chorus_update_task`（`acceptanceCriteriaItems` 是**全量替换**，要带上所有 AC）。
- **reviewer 必须走 async/后台**：前台子代理没有 `mcp`，发不出 VERDICT 评论；派发后用 `bg_wait` 等完成，再从「提案/任务评论」读 VERDICT（完成通知里往往只有摘要）。

## 纪律（违反会造成返工或假绿）

- **单写者**：同一 cwd 同时只允许一个 worker 写；reviewer/scout 只读可并行。并行写必须 worktree 隔离。
- **Scope 例外要留痕**：绕不开的机械改动（如加字段后补 `..Default::default()`、加命令透传胶水）允许做，但必须① 在报告里说明理由与触碰行② 引用先例 commit③ 保持最小。
- **AC 要机器可验证**：「显著变小」→ 量化成「≤ 1/2 并有像素对照」；「无持久化写入」→ 明确到「条目不写、ETag 不覆盖，失败态按既有路径记录」。
- **durable spec 勾选纪律**：只勾有证据的；局限必须写进勾选注（例：Windows 运行时未测）；未交付的**不勾**（评估者最容易在这里过度声明）。
- **结论强度 ≤ 证据**：绝对化措辞（任何/只有/绝不）要么给穷举反证，要么改成可验证表述。
- **诚实三态**：报告里明确区分 已验证 / 未验证 / 无法验证，并给遗留项的可执行核验步骤。
- **子代理任务书**：内联精确路径白名单 + `timeout` + 结果截断 + 「命令慢就跳过」；禁止整树 grep/前台 GUI。

## 收口清单

1. `cargo test --workspace` 全绿 + `cargo clippy --workspace` 无新增；
2. durable spec 勾选（带证据与局限）+ `manual-verification-checklist.md` 新增本批次章节（机械证据 / 实机快照 / 环境限制）；
3. README 同步（工具数、行为、权限模型等）；
4. `chorus_create_report`；
5. 环境清场（kill 自己起的进程、清 /tmp 产物）。
