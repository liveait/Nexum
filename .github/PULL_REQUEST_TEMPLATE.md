## 目标与范围 / Goal and scope

<!-- 问题、验收场景、本次范围和不变量；小改动可直接在这里说明，无需重复建 Issue。 -->
<!-- Problem, acceptance scenarios, scope, and invariants. A small change can be defined here. -->

## 类型 / Type

- [ ] 修复 / Fix
- [ ] 功能 / Feature
- [ ] 重构 / Refactor
- [ ] 文档 / Docs
- [ ] 构建或 CI / Build or CI

## 方案与保护 / Approach and protections

<!-- 说明调用链、失败/取消/重启路径，及从旧路径保留的保护。涉及架构时链接 RFC/ADR。 -->
<!-- Explain call paths, failure/cancel/restart handling, and protections retained from the old path. Link an RFC/ADR when applicable. -->

## 验证证据 / Verification evidence

<!-- 记录实际命令和结果；未运行、仅编译、通过、失败、跳过分别说明。可链接日志。 -->
<!-- Record actual commands/results. Distinguish not run, compiled only, passed, failed, and skipped; log links are welcome. -->

| 命令或场景 / Command or scenario | 结果 / Result | 环境或证据 / Environment or evidence |
| --- | --- | --- |
| | | |

<!-- 原生行为说明 App 验收；低影响文字/样式说明 diff、链接或界面检查，不要求新增测试。 -->
<!-- Include native app checks for native behavior; low-impact text/styles need diff/link/UI inspection, not new tests. -->

## 独立审查 / Independent review

- 被审查的代码 SHA / Reviewed code SHA:
- 审查者及记录 / Reviewer and record:
- 已解决和待处理问题 / Resolved and open findings:
- 合并前 CI 对应的 head / base SHA 与结果 / Pre-merge CI head / base SHA and results:

<!-- 审查后代码变化需复核受影响部分；合并前核对 PR head、审查与 CI。此信息放在 PR 描述/审查记录，不在源码中追写自身 SHA。 -->
<!-- Re-review affected parts after code changes. Before merging, match PR head, review, and CI. Keep SHA evidence in the PR/review record, not self-referential source updates. -->

## 文档与后续 / Documentation and follow-up

<!-- 更新相关中英文文档；区分方案已接受、已实现待验收、已验收。记录验证缺口和后续安排。 -->
<!-- Update relevant docs in both languages; distinguish accepted design, implemented awaiting acceptance, and verified. Record gaps and follow-up. -->

## 合并检查 / Merge checklist

- [ ] 改动聚焦并符合贡献流程 / Focused change following the contribution workflow
- [ ] 当前代码已独立审查，阻断问题与已知回归已解决 / Current code independently reviewed; blockers and known regressions resolved
- [ ] 完整适用门槛已执行且无失败，包含 Server unit/managed/persistence（如适用）/ Full applicable gates pass, including Server unit/managed/persistence where applicable
- [ ] 未以排除测试、弱化断言或抑制错误掩盖回归 / No regressions hidden by excluded tests, weakened assertions, or suppressed errors
- [ ] 审查覆盖当前 PR head，相关 CI 验证该 head 与最新 main 的组合 / Current PR head reviewed and applicable CI tests it with the latest main
- [ ] 文档准确，验证缺口与后续事项已记录 / Accurate docs with verification gaps and follow-up recorded

## 合并后验收 / Post-merge acceptance

<!-- 合并后在 PR 描述或交接记录补充；合并前保留待验收。 -->
<!-- Fill in the PR description or handoff after merging; leave pending before merge. -->

- 实际合并提交 SHA / Actual merged commit SHA:
- main push CI 运行链接及结果 / Main push CI run and results:
- [ ] 该合并 SHA 的必需 job 全部通过；若 main 又更新，最新 head 也已验证 / Required jobs pass on this merged SHA; latest main also verified if it advanced
- [ ] 主分支验收后才标记完成、发布或清理定位所需的 worktree / Completion, publishing, and diagnostic worktree cleanup follow main acceptance

<!-- 失败时暂停后续功能合并和发布，保留日志，定位后修复或评估回滚，并验证恢复后的 main。 -->
<!-- On failure, pause subsequent feature merges/releases, preserve logs, fix or assess a revert, and verify the restored main. -->
