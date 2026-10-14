# Nexum 仓库代理规则 / Repository instructions

阅读[贡献指南](CONTRIBUTING.md)中的统一流程及[开发指南](docs/DEVELOPMENT.md)中的命令。用户直接指示和已有授权优先。
Read [CONTRIBUTING.md](CONTRIBUTING.md) for the shared workflow and [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for commands. Direct user instructions and existing authorization take precedence.

## 任务协作 / Working on a task

- 记录目标、验收场景、范围和基线提交。小改动沿用已有计划或任务消息即可；架构改动先形成 RFC。
  Record the goal, acceptance scenarios, scope, and baseline commit. An existing plan/task message is sufficient for a small change; architectural changes need an RFC first.
- 并行实现使用独立分支/worktree；Codex 分支默认用 `codex/`。保留其他工具的暂存及未暂存工作，不重置或切换其正在使用的 checkout。
  Use separate branches/worktrees for concurrent implementations. Codex-created branches use `codex/` unless specified otherwise. Preserve other tools' staged/unstaged work; never reset or switch their active checkout.
- 一个 PR 解决一个问题。实现者自测，独立审查者固定 SHA 检查行为与实际调用链。
  Keep one problem per PR. The implementer self-tests; an independent reviewer checks behavior and actual call paths at a recorded SHA.
- 代码变更后复核受影响问题和验证。合并前核对审查 head 与最新 main 的组合 CI；base 前进时更新分支触发新验证，重跑旧运行不能证明新 base 已测试。
  Recheck affected findings and verification after code changes. Before merging, match the reviewed PR head and CI testing it with the latest main. Update the branch for fresh CI when the base advances; an old rerun does not verify a new base.

## 正确性与验证 / Correctness and verification

- 禁止通过排除失败测试/crate、增加 ignore、弱化断言或抑制错误修复 CI。更正错误测试需说明原因并保留等效或更强覆盖。
  Never fix CI by excluding failing tests/crates, adding ignores, weakening assertions, or suppressing errors. Correct an invalid test only with an explanation and equivalent or stronger coverage.
- 完整 Rust 门槛包含 Server unit、managed 和 persistence。聚焦测试不能替代完整门槛；命令和平台检查以开发指南为准。
  Complete Rust gates include Server unit, managed, and persistence tests. Focused tests do not replace them; use the Development Guide for commands and platform checks.
- 新路径需直接测试并经消费者验证接线，不按 fixture URL/地址跳过产品执行路径。
  Test new execution paths directly and through their consumers. Do not bypass product paths using fixture URLs/addresses.
- 下载变更保留文件/链接安全、Range/validator 校验、完成凭证、最终进度与 Completed 原子保存、发布前取消及整文件续传进度。替换路径前阅读 ADR 0003、0004、0007。
  Download changes preserve file/symlink safety, Range/validator checks, completion receipts, atomic final progress/Completed persistence, cancellation before publication, and whole-file resume progress. Review ADRs 0003, 0004, and 0007 before replacing these paths.
- 进程和失败测试使用私有临时数据/下载目录。原生 Desktop 行为需要原生 App 验收，浏览器预览不能代替。
  Use private temporary data/download directories for process and failure tests. Native Desktop behavior requires a native app check; browser preview is insufficient.
- 报告实际命令/结果、失败、跳过项及平台缺口。区分方案接受、代码实现与行为验收。
  Report actual commands/results, including failures, skipped checks, and platform gaps. Distinguish accepted design, implemented code, and verified behavior.
- 低影响文字/样式修改不添加仅复述文字或实现的测试。
  Do not add tests that mirror wording or implementation for low-impact text/style edits.

## 文档 / Documentation

- 每个主题保留一个标准文件名的双语 `.md`，标题、段落、列表及表格均中文在前、英文在后，共用相同示例；合并时更新链接/锚点，不再新建平行 `.zh-CN.md` 副本。
  Keep one bilingual `.md` per topic using the standard filename. Place Chinese before English in headings, paragraphs, lists, and tables; share identical examples, and update links/anchors when consolidating. Do not create parallel `.zh-CN.md` copies.
- 两种语言内容保持准确一致，验收条件通过后才标记里程碑完成。
  Keep both languages accurate and consistent. Mark a milestone complete only when its acceptance criteria are verified.

## 交付 / Delivery

- 阻断标准针对本次改动及依赖。纯文档或恢复门槛的修复可在明确记录基线缺陷后独立合并，不代表基线行为验收；后续功能合并/发布仍需回归修复和完整门槛通过。
  Apply blocking criteria to the change and its dependencies. Documentation-only or gate-restoration fixes may merge with baseline defects explicitly recorded; this does not accept baseline behavior. Subsequent feature merges/releases still wait for regression fixes and complete passing gates.
- 合并前解决行为回归和阻断问题。未解决 P0/P1 阻止合并；P2 修复或记录接受原因与后续安排。
  Resolve behavior regressions and blocking findings before merging. Open P0/P1 findings block merging; fix P2 findings or record acceptance and follow-up.
- 已有用户授权时，合格 PR 直接合并，不重复确认。随后验证实际合并 SHA 的 main push CI；必需 job 全通过后才完成、发布或清理定位所需 worktree，main 再前进时也检查最新 head。
  With existing user authorization, merge a qualifying PR without asking again. Then verify the main push CI for its actual merged SHA. All required jobs must pass before completion, publishing, or diagnostic worktree cleanup; also check latest main if it advances.
- 合并后 CI 失败时暂停功能合并/发布，保留证据，定位后修复或评估回滚，并复验恢复后的 main。缺失/取消/跳过必需检查不算成功；发布/部署另遵循对应指示。
  If post-merge CI fails, pause feature merges/releases, preserve evidence, find the cause, and fix or assess a revert before verifying restored main. Missing/cancelled/skipped required checks are not success. Follow separate release/deployment instructions.
- 保留产物，用 App 工具归档已完成 managed worktree，再核对归档记录、目录移除和 Git worktree 列表。只同步空闲且干净的 checkout。
  Preserve artifacts, archive completed managed worktrees with the app tool, and verify the archive record, directory removal, and Git worktree list. Synchronize only idle, clean checkouts.
- 本文件不会配置 GitHub 分支保护，声称远端强制生效前必须核实。
  These instructions do not configure GitHub branch protection; verify remote enforcement before claiming it.
