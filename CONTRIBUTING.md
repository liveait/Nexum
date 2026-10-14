<a id="contributing"></a>
<a id="贡献指南"></a>

# 贡献指南 / Contributing

Nexum 由个人维护，可由多个编码工具协作。采用以下统一流程；维护者直接给出的任务与授权优先。小改动直接 PR，大改动先形成方案，架构改动先 RFC。

Nexum has an individual maintainer and may use several coding tools. Follow this shared workflow; direct task instructions and authorization from the maintainer take precedence. Small changes can go directly to a PR, larger changes need a plan, and architectural changes need an RFC first.

每个文档主题只保留一个标准文件名的双语 `.md`。标题、段落、列表及表格都采用中文在前、英文在后的顺序，共用相同的命令和代码示例；新增或更新文档时同时维护两种语言，不再创建平行的中文副本。

Keep one bilingual `.md` with a standard filename per topic. Place Chinese before English in headings, paragraphs, lists, and tables, and share identical commands and code examples. Maintain both languages when adding or changing docs; do not create parallel Chinese copies.

<a id="development-workflow"></a>
<a id="开发流程"></a>

## 开发流程 / Development Workflow

| 阶段<br>Stage | 需要完成的工作<br>Work required | 进入下一阶段的条件<br>Condition for proceeding |
| --- | --- | --- |
| 1. 定义任务<br>1. Define the task | 写明目标、涉及模块、验收场景与本次范围；沿用已有计划或任务消息即可<br>State the goal, modules, acceptance scenarios, and scope; an existing plan or task message is sufficient | 结果可验证，范围清晰<br>The outcome is verifiable and the scope is clear |
| 2. 隔离实现<br>2. Isolate implementation | 从已确认的基线创建分支和独立 worktree；一个 PR 解决一个问题<br>Create a branch and separate worktree from a confirmed baseline; solve one problem per PR | 基线提交、工作目录及文件负责人明确<br>Record the baseline commit, workspace, and file ownership |
| 3. 编码与自测<br>3. Implement and self-test | 小步实现，先运行相关测试；行为修复用能在旧代码上失败的复现测试<br>Work in small steps and run relevant tests first; reproduce behavioral bugs with tests that fail before the fix | 修复场景通过，已有保护仍在<br>The scenario passes and existing protections remain intact |
| 4. 独立审查<br>4. Review independently | 由未承担该改动实现的审查者检查 diff、调用链和测试；记录被审查 SHA<br>A reviewer who did not implement the change checks the diff, call paths, and tests; record the reviewed SHA | 已明确问题、验证结果及覆盖缺口<br>Findings, verification results, and coverage gaps are explicit |
| 5. 修复与复验<br>5. Fix and verify | 编码者修复，审查者复核；运行相应完整检查<br>The author fixes findings, the reviewer verifies them, and the applicable full checks run | 已知回归和阻断问题解决，检查有实际结果<br>Known regressions and blocking findings are resolved, with actual results |
| 6. 合并<br>6. Merge | 核对审查的 PR head，以及与最新 `main` 组合的 CI<br>Match the reviewed PR head and CI for its combination with the latest `main` | 满足合并条件后，按维护者已有授权合并<br>Meet merge conditions and follow the maintainer's existing authorization |
| 7. 主分支复验<br>7. Verify main | 记录实际合并提交 SHA，检查该 SHA 的 `main` push CI<br>Record the actual merged commit SHA and check its `main` push CI | 必需 job 全部通过，才算交付完成<br>All required jobs pass before delivery is complete |
| 8. 同步与清理<br>8. Synchronize and clean up | 安全同步空闲 checkout，归档完成的 worktree，核对目录和分支<br>Safely synchronize idle checkouts, archive completed worktrees, and verify directories and branches | 主分支复验通过，保留所需产物，并确认清理实际完成<br>Main verification passes, artifacts are preserved, and cleanup actually finished |

任务说明可写在 PR、Issue、计划或当前任务消息中，不要求为每个小修复制创建 Issue/RFC。架构和跨模块行为修改应先更新方案；RFC 范围见下文。

Record the task in a PR, Issue, plan, or current task message; do not create duplicate Issues or RFCs for every small fix. Update the plan before architectural or cross-module behavior changes. See the RFC categories below.

<a id="collaboration-across-tools"></a>
<a id="多工具协作"></a>

### 多工具协作 / Collaboration Across Tools

- 每项任务指定一个实现者和一个独立审查者。编码者可以自测，但自测不能替代独立审查。审查者可以是另一个模型；重要结论仍需代码和运行证据。<br>
  Assign an implementer and an independent reviewer for each task. Self-testing is required but does not replace independent review. Another model may review, but material conclusions still require code and runtime evidence.

- 多个工具并行实现时使用不同分支/worktree。对同一文件的修改由一个实现者汇总；其他工具提供分析或补丁。<br>
  Parallel implementers use separate branches/worktrees. One implementer integrates changes to a shared file; others provide analysis or patches.

- 开始前读取仓库 `AGENTS.md`、本指南、[开发指南](docs/DEVELOPMENT.md)及相关计划/ADR。明确基线 SHA，保留已有未提交和已暂存工作。<br>
  Read repository `AGENTS.md`, this guide, the [Development Guide](docs/DEVELOPMENT.md), and relevant plans/ADRs before starting. Identify the baseline SHA and preserve existing unstaged and staged work.

- 审查后如代码继续变化，重新检查受影响部分并更新审查 SHA；相关测试与 CI 也需覆盖新 head。其他分支上的修复不能作为当前 PR 的验证结果。<br>
  If code changes after review, review the affected parts again and update the reviewed SHA. Relevant tests and CI must cover the new head. A fix on another branch is not evidence for this PR.

- 使用能力满足任务的工具分工，不以模型名或“CI 通过”替代质量判断。文件安全、数据库/状态机、网络传输和架构改造需要独立检查关键不变量。<br>
  Assign tools according to the task. Model names and green CI alone do not prove correctness. File safety, persistence/state machines, networking, and architecture changes require an independent check of their invariants.

<a id="verification-by-change-type"></a>
<a id="按改动选择验证"></a>

### 按改动选择验证 / Verification by Change Type

| 改动<br>Change | 本地验证及审查重点<br>Local verification and review focus |
| --- | --- |
| 文字、静态样式或低影响布局<br>Text, static styles, or low-impact layout | diff、链接或实际界面；不为文字或纯布局添加重复实现的测试<br>Diff, links, or actual UI; do not add tests that mirror wording or layout implementation |
| 普通功能或缺陷修复<br>Ordinary features or bug fixes | 受影响测试、异常路径；Rust 行为变化运行完整 Rust 检查，Desktop/Extension 运行对应包检查<br>Relevant tests and failure paths; Rust behavior changes run all Rust checks, while Desktop/Extension changes run their package checks |
| Engine、文件操作、持久化、调度、协议或并发<br>Engines, file operations, persistence, scheduling, protocols, or concurrency | 上述检查，加受控真实执行/进程测试，验证失败、取消、重启和文件完整性；架构变化按 RFC 流程<br>The above plus controlled execution/process tests covering failure, cancellation, restart, and file integrity; architectural changes follow the RFC process |
| Desktop 原生行为或打包<br>Native Desktop behavior or packaging | 对应前后端检查、`.app` 构建和包验证；涉及原生命令、设置或生命周期时操作原生 App 验收<br>Applicable frontend/backend checks, `.app` build and bundle verification; validate native commands, settings, or lifecycle changes in the native app |

完整命令统一维护在[开发指南](docs/DEVELOPMENT.md#rust-检查)。本地环境无法执行某项时，记录缺口并取得对应 CI 或适用平台的结果。浏览器预览只证明前端表现，不能代替原生 App 行为验收。

Keep command definitions in the [Development Guide](docs/DEVELOPMENT.md#rust-checks). If a local environment cannot run a check, record the gap and obtain the corresponding CI or platform result. Browser preview demonstrates frontend behavior; native app acceptance still requires the native app.

<a id="testing-and-ci-rules"></a>
<a id="测试与-ci-的规则"></a>

### 测试与 CI 的规则 / Testing and CI Rules

- 完整 Rust 门槛必须包含 Server 单元、`managed` 与 `persistence` 测试。聚焦测试用于快速反馈，不能替代完整门槛。<br>
  The full Rust gate must include Server unit, `managed`, and `persistence` tests. Focused tests give fast feedback and do not replace this gate.

- 禁止通过排除 crate/测试、增加 `ignore`、弱化断言或屏蔽错误来让有回归的 CI 通过。测试本身有误时，说明旧断言为何不符合约定，并提供等效或更强的行为验证。<br>
  Do not make failing CI green by excluding crates/tests, adding `ignore`, weakening assertions, or suppressing errors. If a test is wrong, explain why its old assertion conflicts with the contract and provide equivalent or stronger behavior coverage.

- 正式删除或替换覆盖、调整检查范围和例外时，必须单独说明理由与替代保护，经过独立审查；不能用这些修改宣称相关功能已验收。<br>
  Intentional removal/replacement of coverage, changes to gate scope, and exceptions require an explicit rationale, replacement protection, and independent review. Such changes cannot be used to claim that affected behavior has passed acceptance.

- 修复 CI 时保留失败信息并定位原因。环境或时序问题可以改进 fixture、隔离资源或调整执行方式，同时保留断言覆盖的行为。<br>
  Preserve failure information and identify its cause when fixing CI. Improve fixtures, resource isolation, or execution mode for environment/timing issues while retaining the asserted behavior.

- 新执行路径必须有直接调用它的测试，并通过实际消费者（例如 Server）验证接线。仅测试遗留方法、拆段辅助函数或探测函数不足以验证新传输路径。<br>
  Test new execution paths directly and through their consumers, such as the Server. Tests of legacy methods, segment splitting, or probing alone do not verify a new transfer path.

- 禁止为了测试而按 fixture 的地址/URL 跳过产品执行路径。通过可控服务和明确依赖注入构造场景。<br>
  Do not bypass a product path based on a fixture address/URL. Use controlled services and explicit dependency injection to construct scenarios.

- 记录命令、实际通过/失败/跳过结果与环境。未运行、仅编译、失败、已通过分别表述；有失败时先修复原因。<br>
  Record commands, actual pass/fail/skip results, and the environment. Distinguish not run, compiled only, failed, and passed; resolve failures first.

<a id="nexum-invariants"></a>
<a id="nexum-关键不变量"></a>

### Nexum 关键不变量 / Nexum Invariants

涉及下载路径的重构或新 Engine 必须保留并验证：

A download refactor or new engine must preserve and verify:

1. **文件保护**：保留已有目标和无关文件；拒绝不安全的 staging/符号链接，不把固定临时路径当作可信普通文件。<br>
   **File protection:** preserve existing destinations and unrelated files; reject unsafe staging files/symlinks and never assume a fixed temporary path is a trusted regular file.

2. **数据完整性**：验证 Range 起止/总长、响应长度及 ETag/Last-Modified 身份；内容缺失、错位或源变化时不得标记完成。<br>
   **Data integrity:** validate Range bounds/total, response length, and ETag/Last-Modified identity; missing, misplaced, or changed content cannot be marked completed.

3. **完成恢复**：发布前保留完成凭证，最终进度与 `Completed` 原子保存；进程退出或 SQLite 失败后可按凭证恢复，不能把替换文件认作已完成下载。<br>
   **Completion recovery:** retain completion receipts before publication and persist final progress plus `Completed` atomically; recover after process exit or SQLite failure without treating a replacement file as the completed download.

4. **控制与资源**：暂停后进度稳定；提交前取消有效；失败、删除与重试正确释放调度/目标槽位，并保留可操作的错误。<br>
   **Control and resources:** progress is stable while paused; cancellation before publication takes effect; failure, removal, and retries release scheduling/destination slots correctly and preserve actionable errors.

5. **续传与能力声明**：续传累计原偏移，进度代表整个文件；宣称的回退、能力路由和跨重启恢复必须有实现与直接验收证据。<br>
   **Resume and capabilities:** include the saved offset in progress, use whole-file totals, and provide implementation plus direct acceptance evidence for claimed fallback, capability routing, and restart recovery.

相关约定见 [ADR 0003](docs/decisions/0003-http-transfer-controls.md)、[ADR 0004](docs/decisions/0004-cross-restart-http-resume.md)及 [ADR 0007](docs/decisions/0007-http-completion-reconciliation.md)。重构前先列出旧路径提供的保护，禁止仅搬运成功路径。

See [ADR 0003](docs/decisions/0003-http-transfer-controls.md), [ADR 0004](docs/decisions/0004-cross-restart-http-resume.md), and [ADR 0007](docs/decisions/0007-http-completion-reconciliation.md). List the old path's protections before refactoring; carry each one into the new path.

<a id="merge-and-completion-conditions"></a>
<a id="合并与完成的定义"></a>

### 合并与完成的定义 / Merge and Completion Conditions

阻断标准针对本次改动及其依赖。纯文档或恢复验证门槛的修复可在明确记录基线缺陷后独立合并；这不代表基线功能已验收，后续功能合并和发布仍需等待回归修复及完整门槛通过。

Blocking criteria apply to this change and its dependencies. Documentation-only changes or fixes restoring verification gates may merge independently with explicitly recorded baseline defects. This does not accept the affected baseline behavior; subsequent feature merges and releases still wait for regression fixes and complete passing gates.

合并前应满足：

Before merging:

- 改动聚焦，独立审查覆盖当前 head；已知行为回归和阻断问题已修复。<br>
  Keep the change focused and independently reviewed at the current head. Resolve known behavior regressions and blocking findings.

- 所需本地/平台验证有记录，审查覆盖当前 PR head，并且该 head 与最新 `main` 组合的相关 CI 全部通过；P0/P1 未解决时阻止合并，P2 应修复或明确记录接受原因和后续安排。<br>
  Record required local/platform verification, review the current PR head, and require passing applicable CI for that head combined with the latest `main`. Open P0/P1 findings block merging; fix P2 findings or explicitly document acceptance and follow-up.

- 相关计划、ADR、开发说明和变更日志的中英文内容与实际交付一致。“方案已接受”“已实现待验收”“已验收”分别标注，只有达到验收条件才能标记里程碑完成。<br>
  Keep both languages in relevant plans, ADRs, development notes, and changelogs accurate. Distinguish accepted design, implemented awaiting acceptance, and verified. Mark a milestone complete only after its acceptance criteria are met.

维护者已经授权自动合并时，满足条件后直接合并，不重复请求确认。没有授权时按维护者的指示处理。新功能发布、公证和部署遵循相应发布流程，不因 PR 合并自动视为完成。

When the maintainer has already authorized automatic merging, merge once these conditions are met without requesting confirmation again. Otherwise follow the maintainer's instructions. Releases, notarization, and deployment follow their own release process; merging a PR does not complete them.

<a id="verify-ci-before-and-after-merging"></a>
<a id="合并前与合并后都要检查-ci"></a>

#### 合并前与合并后都要检查 CI / Verify CI Before and After Merging

1. **合并前**：确认 PR head 与审查记录一致，CI 验证的是该 head 与最新 base 的组合。GitHub `pull_request` 通常检出测试合并引用；需要核对实际 checkout 和 base，不能只看源分支上某次绿色结果。`main` 已前进时，通过更新分支触发新的验证；重跑旧运行不代表测试了新 base。<br>
   **Before merge:** match the PR head with the reviewed SHA and ensure CI tests that head combined with the latest base. GitHub `pull_request` normally checks out a test merge ref; verify the actual checkout and base rather than trusting any green run on the source branch. If `main` has advanced, update the branch to trigger fresh validation; rerunning an old run does not establish coverage of the new base.

2. **合并后**：从 PR 取得实际 merge/squash/rebase 结果的提交 SHA，找到 `main` 分支、`push` 事件且 `headSha` 为该值的 CI，等待 `Rust` 和 `Desktop macOS` 等必需 job 全部通过。缺失、排队、取消、失败、必要 job 被跳过，都不算通过；不能取同名分支上一条旧的成功运行。<br>
   **After merge:** obtain the actual commit produced by merge/squash/rebase from the PR. Find CI with branch `main`, event `push`, and `headSha` matching that commit. Wait for all required jobs, including `Rust` and `Desktop macOS`, to pass. Missing, queued, cancelled, failed, or skipped required jobs do not count as success. Do not substitute an older successful run on the same branch.

3. **完成条件**：合并后 CI 通过之前，状态为“已合并，等待主分支验收”，不标记里程碑完成、不发布新包、不清理用于定位问题的 worktree/证据。若 `main` 又更新，还应核对最新 head 的门槛，不能用旧提交的成功替代最新结果。<br>
   **Completion:** until post-merge CI passes, report “merged, awaiting main acceptance.” Do not complete the milestone, publish a new package, or discard worktrees/evidence needed for diagnosis. If `main` advances again, also check the latest head's gates; an older commit's success does not verify the latest state.

4. **失败处理**：暂停后续功能合并和发布，比较合并前后的代码、workflow、环境和日志；保留失败证据。确认是合并改动造成时，修复或评估后回滚该改动，并重新验证恢复后的 `main` CI。重跑成功只适用于已定位的临时环境故障，不能掩盖不稳定或实际回归。<br>
   **Failure:** pause subsequent feature merges and releases. Compare code, workflow, environments, and logs before and after merging, preserving failure evidence. If the merged change caused the failure, fix it or assess and revert it, then verify CI on the restored `main`. A successful rerun is sufficient only for an identified temporary environment fault; it must not conceal instability or regressions.

这能缩小合并后的意外窗口；不能承诺不同触发条件和环境永远不出问题。配置 `main` required checks 和要求分支最新可减少组合风险；采用 merge queue 前还需保证 workflow 支持 `merge_group` 事件。

These checks reduce surprises after merging but cannot guarantee identical behavior across all events and environments. Required checks and requiring an up-to-date branch reduce integration risk. Configure workflow support for `merge_group` before adopting a merge queue.

清理时先保留仍需使用的 App、截图和报告。通过工具归档 managed worktree 后，检查归档记录、实际目录和 `git worktree list`；不能仅根据返回的 queued/success 宣称清理完成。正在被其他工具使用或存在未提交改动的 checkout 不切分支、不重置、不拉取覆盖；它的同步留到空闲后处理。

Preserve needed apps, screenshots, and reports before cleanup. After archiving a managed worktree, verify its archive record, directory, and `git worktree list`; a queued/success response alone is not proof. Do not switch branches, reset, or pull over a checkout being used by another tool or containing uncommitted changes; synchronize it when idle.

仓库 `AGENTS.md` 和 PR 模板是协作规则，不能技术性阻止所有工具违反流程。建议在 GitHub 配置 `main` 的 PR 合并规则和当前 `Rust`、`Desktop macOS` required checks；个人维护无需额外的人工批准人数。分支保护与 required checks 需在 GitHub 另行配置，本文档不表示它们已启用，必需检查自身也必须保留完整测试覆盖。

Repository `AGENTS.md` and the PR template are collaboration rules, not technical enforcement for every tool. Recommended GitHub settings require PRs and the current `Rust` and `Desktop macOS` status checks for `main`; individual maintenance does not need an additional human approval count. Configure branch protection and required checks separately in GitHub. This document does not claim they are enabled, and required checks themselves must retain full test coverage.

<a id="handoff-record"></a>
<a id="交接信息"></a>

### 交接信息 / Handoff Record

在 PR 或交接消息中提供：

Include in the PR or handoff message:

```text
目标与验收： / Goal and acceptance:
基线 SHA / 分支 / worktree： / Baseline SHA / branch / worktree:
改动范围及关键保护： / Scope and key protections:
相关测试命令与实际结果： / Test commands and actual results:
独立审查覆盖的 SHA / 已解决与待处理问题： / Reviewed SHA / resolved and open findings:
合并前 CI 覆盖的 head / base SHA 与结果： / Pre-merge CI head / base SHA and results:
实际合并 SHA / main push CI 结果（合并后补充）： / Actual merged SHA / main push CI results (add after merging):
文档状态 / 平台验证缺口 / 后续事项： / Documentation status / platform gaps / follow-up:
```

<a id="when-an-rfc-is-required"></a>
<a id="什么情况需要-rfc"></a>

## 什么情况需要 RFC / When an RFC Is Required

以下类型的改动应先提出 RFC：

Use an RFC for:

- 核心架构调整<br>
  Core architecture changes

- Nexum Protocol 的新增或破坏性修改<br>
  New or breaking Nexum Protocol changes

- 数据模型或数据库迁移<br>
  Data model or database migrations

- 核心任务状态机变化<br>
  Core task state-machine changes

- 插件权限模型变化<br>
  Plugin permission model changes

- Engine Adapter 抽象变化<br>
  Engine Adapter abstraction changes

- 可能影响多个模块的 Breaking Change<br>
  Breaking changes affecting multiple modules

保持 UI 逻辑与 Core 分离，避免将具体 Engine 的实现细节放入 Domain Model。依赖应解决具体需要，错误应保留可操作的上下文；Core 行为尽量在无网络环境下测试，网络和 Engine 行为使用受控集成测试。将公开 crate API 以及 Protocol、Storage、Plugin 合约视为兼容性边界，记录有意的破坏性变更。

Keep UI logic separate from Core and concrete engine details out of the domain model. Add dependencies for concrete needs and retain actionable error context. Keep Core testable without network access and use controlled integration tests for networking and engines. Treat public crate APIs and protocol, storage, and plugin contracts as compatibility boundaries and document deliberate breaking changes.

<a id="submission-standards"></a>
<a id="提交规范"></a>

## 提交规范 / Submission Standards

提交信息使用 Conventional Commits：`type: description`。类型可为 `feat`、`fix`、`docs`、`style`、`refactor`、`test` 或 `chore`。类型后使用冒号和空格；描述使用英文且末尾不加句号，与现有提交历史保持一致。

Use Conventional Commits: `type: description`. Types include `feat`, `fix`, `docs`, `style`, `refactor`, `test`, and `chore`. Place a colon and space after the type. Write the description in English without a trailing period, following existing history.

<a id="technical-discussion"></a>
<a id="技术讨论"></a>

## 技术讨论 / Technical Discussion

可以直接、充分地讨论技术方案，也可以提出反对意见。请针对代码、设计和证据讨论，不针对贡献者本人。

Strong technical disagreement is welcome. Discuss code, design, evidence, and trade-offs rather than contributors.

<a id="security"></a>
<a id="安全问题"></a>

## 安全问题 / Security

不要在公开 Issue 中披露尚未修复的安全漏洞。请按照 [SECURITY.md](SECURITY.md) 中的流程报告。

Do not disclose unfixed security vulnerabilities in public Issues. Follow [SECURITY.md](SECURITY.md) for reporting guidance.
