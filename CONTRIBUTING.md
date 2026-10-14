# Contributing

Nexum has an individual maintainer and may use several coding tools. Follow this shared workflow; direct task instructions and authorization from the maintainer take precedence. Small changes can go directly to a PR, larger changes need a plan, and architectural changes need an RFC first.

## Development Workflow

| Stage | Work required | Condition for proceeding |
| --- | --- | --- |
| 1. Define the task | State the goal, modules, acceptance scenarios, and scope; an existing plan or task message is sufficient | The outcome is verifiable and the scope is clear |
| 2. Isolate implementation | Create a branch and separate worktree from a confirmed baseline; solve one problem per PR | Record the baseline commit, workspace, and file ownership |
| 3. Implement and self-test | Work in small steps and run relevant tests first; reproduce behavioral bugs with tests that fail before the fix | The scenario passes and existing protections remain intact |
| 4. Review independently | A reviewer who did not implement the change checks the diff, call paths, and tests; record the reviewed SHA | Findings, verification results, and coverage gaps are explicit |
| 5. Fix and verify | The author fixes findings, the reviewer verifies them, and the applicable full checks run | Known regressions and blocking findings are resolved, with actual results |
| 6. Merge | Match the reviewed PR head and CI for its combination with the latest `main` | Meet merge conditions and follow the maintainer's existing authorization |
| 7. Verify main | Record the actual merged commit SHA and check its `main` push CI | All required jobs pass before delivery is complete |
| 8. Synchronize and clean up | Safely synchronize idle checkouts, archive completed worktrees, and verify directories and branches | Main verification passes, artifacts are preserved, and cleanup actually finished |

Record the task in a PR, Issue, plan, or current task message; do not create duplicate Issues or RFCs for every small fix. Update the plan before architectural or cross-module behavior changes. See the RFC categories below.

### Collaboration Across Tools

- Assign an implementer and an independent reviewer for each task. Self-testing is required but does not replace independent review. Another model may review, but material conclusions still require code and runtime evidence.
- Parallel implementers use separate branches/worktrees. One implementer integrates changes to a shared file; others provide analysis or patches.
- Read repository `AGENTS.md`, this guide, the [Development Guide](docs/DEVELOPMENT.md), and relevant plans/ADRs before starting. Identify the baseline SHA and preserve existing unstaged and staged work.
- If code changes after review, review the affected parts again and update the reviewed SHA. Relevant tests and CI must cover the new head. A fix on another branch is not evidence for this PR.
- Assign tools according to the task. Model names and green CI alone do not prove correctness. File safety, persistence/state machines, networking, and architecture changes require an independent check of their invariants.

### Verification by Change Type

| Change | Local verification and review focus |
| --- | --- |
| Text, static styles, or low-impact layout | Diff, links, or actual UI; do not add tests that mirror wording or layout implementation |
| Ordinary features or bug fixes | Relevant tests and failure paths; Rust behavior changes run all Rust checks, while Desktop/Extension changes run their package checks |
| Engines, file operations, persistence, scheduling, protocols, or concurrency | The above plus controlled execution/process tests covering failure, cancellation, restart, and file integrity; architectural changes follow the RFC process |
| Native Desktop behavior or packaging | Applicable frontend/backend checks, `.app` build and bundle verification; validate native commands, settings, or lifecycle changes in the native app |

Keep command definitions in the [Development Guide](docs/DEVELOPMENT.md#rust-checks). If a local environment cannot run a check, record the gap and obtain the corresponding CI or platform result. Browser preview demonstrates frontend behavior; native app acceptance still requires the native app.

### Testing and CI Rules

- The full Rust gate must include Server unit, `managed`, and `persistence` tests. Focused tests give fast feedback and do not replace this gate.
- Do not make failing CI green by excluding crates/tests, adding `ignore`, weakening assertions, or suppressing errors. If a test is wrong, explain why its old assertion conflicts with the contract and provide equivalent or stronger behavior coverage.
- Intentional removal/replacement of coverage, changes to gate scope, and exceptions require an explicit rationale, replacement protection, and independent review. Such changes cannot be used to claim that affected behavior has passed acceptance.
- Preserve failure information and identify its cause when fixing CI. Improve fixtures, resource isolation, or execution mode for environment/timing issues while retaining the asserted behavior.
- Test new execution paths directly and through their consumers, such as the Server. Tests of legacy methods, segment splitting, or probing alone do not verify a new transfer path.
- Do not bypass a product path based on a fixture address/URL. Use controlled services and explicit dependency injection to construct scenarios.
- Record commands, actual pass/fail/skip results, and the environment. Distinguish not run, compiled only, failed, and passed; resolve failures first.

### Nexum Invariants

A download refactor or new engine must preserve and verify:

1. **File protection:** preserve existing destinations and unrelated files; reject unsafe staging files/symlinks and never assume a fixed temporary path is a trusted regular file.
2. **Data integrity:** validate Range bounds/total, response length, and ETag/Last-Modified identity; missing, misplaced, or changed content cannot be marked completed.
3. **Completion recovery:** retain completion receipts before publication and persist final progress plus `Completed` atomically; recover after process exit or SQLite failure without treating a replacement file as the completed download.
4. **Control and resources:** progress is stable while paused; cancellation before publication takes effect; failure, removal, and retries release scheduling/destination slots correctly and preserve actionable errors.
5. **Resume and capabilities:** include the saved offset in progress, use whole-file totals, and provide implementation plus direct acceptance evidence for claimed fallback, capability routing, and restart recovery.

See [ADR 0003](docs/decisions/0003-http-transfer-controls.md), [ADR 0004](docs/decisions/0004-cross-restart-http-resume.md), and [ADR 0007](docs/decisions/0007-http-completion-reconciliation.md). List the old path's protections before refactoring; carry each one into the new path.

### Merge and Completion Conditions

Blocking criteria apply to this change and its dependencies. Documentation-only changes or fixes restoring verification gates may merge independently with explicitly recorded baseline defects. This does not accept the affected baseline behavior; subsequent feature merges and releases still wait for regression fixes and complete passing gates.

Before merging:

- Keep the change focused and independently reviewed at the current head. Resolve known behavior regressions and blocking findings.
- Record required local/platform verification, review the current PR head, and require passing applicable CI for that head combined with the latest `main`. Open P0/P1 findings block merging; fix P2 findings or explicitly document acceptance and follow-up.
- Keep both language versions of relevant plans, ADRs, development notes, and changelogs accurate. Distinguish accepted design, implemented awaiting acceptance, and verified. Mark a milestone complete only after its acceptance criteria are met.

When the maintainer has already authorized automatic merging, merge once these conditions are met without requesting confirmation again. Otherwise follow the maintainer's instructions. Releases, notarization, and deployment follow their own release process; merging a PR does not complete them.

#### Verify CI Before and After Merging

1. **Before merge:** match the PR head with the reviewed SHA and ensure CI tests that head combined with the latest base. GitHub `pull_request` normally checks out a test merge ref; verify the actual checkout and base rather than trusting any green run on the source branch. If `main` has advanced, update the branch to trigger fresh validation; rerunning an old run does not establish coverage of the new base.
2. **After merge:** obtain the actual commit produced by merge/squash/rebase from the PR. Find CI with branch `main`, event `push`, and `headSha` matching that commit. Wait for all required jobs, including `Rust` and `Desktop macOS`, to pass. Missing, queued, cancelled, failed, or skipped required jobs do not count as success. Do not substitute an older successful run on the same branch.
3. **Completion:** until post-merge CI passes, report “merged, awaiting main acceptance.” Do not complete the milestone, publish a new package, or discard worktrees/evidence needed for diagnosis. If `main` advances again, also check the latest head's gates; an older commit's success does not verify the latest state.
4. **Failure:** pause subsequent feature merges and releases. Compare code, workflow, environments, and logs before and after merging, preserving failure evidence. If the merged change caused the failure, fix it or assess and revert it, then verify CI on the restored `main`. A successful rerun is sufficient only for an identified temporary environment fault; it must not conceal instability or regressions.

These checks reduce surprises after merging but cannot guarantee identical behavior across all events and environments. Required checks and requiring an up-to-date branch reduce integration risk. Configure workflow support for `merge_group` before adopting a merge queue.

Preserve needed apps, screenshots, and reports before cleanup. After archiving a managed worktree, verify its archive record, directory, and `git worktree list`; a queued/success response alone is not proof. Do not switch branches, reset, or pull over a checkout being used by another tool or containing uncommitted changes; synchronize it when idle.

Repository `AGENTS.md` and the PR template are collaboration rules, not technical enforcement for every tool. Recommended GitHub settings require PRs and the current `Rust` and `Desktop macOS` status checks for `main`; individual maintenance does not need an additional human approval count. Configure branch protection and required checks separately in GitHub. This document does not claim they are enabled, and required checks themselves must retain full test coverage.

### Handoff Record

Include in the PR or handoff message:

```text
Goal and acceptance:
Baseline SHA / branch / worktree:
Scope and key protections:
Test commands and actual results:
Reviewed SHA / resolved and open findings:
Pre-merge CI head / base SHA and results:
Actual merged SHA / main push CI results (add after merging):
Documentation status / platform gaps / follow-up:
```

## When an RFC Is Required

Use an RFC for:

- Core architecture changes
- New or breaking Nexum Protocol changes
- Data model or database migrations
- Core task state-machine changes
- Plugin permission model changes
- Engine Adapter abstraction changes
- Breaking changes affecting multiple modules

Keep UI logic separate from Core and concrete engine details out of the domain model. Add dependencies for concrete needs and retain actionable error context. Keep Core testable without network access and use controlled integration tests for networking and engines. Treat public crate APIs and protocol, storage, and plugin contracts as compatibility boundaries and document deliberate breaking changes.

## Submission Standards

Use Conventional Commits: `type: description`. Types include `feat`, `fix`, `docs`, `style`, `refactor`, `test`, and `chore`. Place a colon and space after the type. Write the description in English without a trailing period, following existing history.

## Technical Discussion

Strong technical disagreement is welcome. Discuss code, design, evidence, and trade-offs rather than contributors.

## Security

Do not disclose unfixed security vulnerabilities in public Issues. Follow [SECURITY.md](SECURITY.md) for reporting guidance.

For the Chinese version, see [CONTRIBUTING.zh-CN.md](CONTRIBUTING.zh-CN.md).
