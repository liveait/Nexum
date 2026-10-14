# Nexum repository instructions

Read [CONTRIBUTING.md](CONTRIBUTING.md) (or its [Chinese version](CONTRIBUTING.zh-CN.md)) for the development workflow and [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for commands. Direct instructions and existing authorization from the user take precedence.

## Working on a task

- Record the goal, acceptance scenarios, scope, and baseline commit. Use an existing plan or task message for small changes; architectural changes need an RFC first.
- Use a separate branch/worktree for each concurrent implementation. For Codex-created branches, use `codex/` unless the user specifies another name. Preserve other tools' staged and unstaged work; never reset or switch their active checkout.
- Keep one problem per PR. The implementer self-tests; an independent reviewer checks the changed behavior and actual call paths at a recorded SHA.
- Recheck affected findings and verification when code changes after review. Before merging, match the reviewed PR head and CI testing its combination with the latest main. Update the branch for fresh CI when the base advances; an old rerun does not verify a new base.

## Correctness and verification

- Never fix CI by excluding failing crates/tests, adding ignores, weakening assertions, or suppressing errors. Correct an invalid test only with an explanation and equivalent or stronger behavior coverage.
- The complete Rust gate includes Server unit, managed, and persistence tests. Focused tests do not replace it. Check commands and required platform verification in the Development Guide.
- Test a new execution path directly and through its consumer. Do not bypass product paths using fixture URLs/addresses.
- Download changes must preserve file/symlink safety, Range and validator checks, completion receipts, atomic final progress/Completed persistence, cancellation before publication, and whole-file resume progress. Review ADRs 0003, 0004, and 0007 before replacing these paths.
- Use private temporary data/download directories for process and failure tests. Native Desktop behavior requires a native app check; browser preview alone is insufficient.
- Report commands and actual results, including failures, skipped checks, and platform gaps. Do not label a plan accepted, code implemented, or behavior verified as interchangeable states.
- Do not add tests that mirror wording or implementation for low-impact text/style edits.

## Delivery

- Apply blocking criteria to the change and its dependencies. Documentation-only changes or fixes restoring verification gates can merge independently with baseline defects explicitly recorded; this does not accept baseline behavior. Subsequent feature merges/releases still wait for regression fixes and complete passing gates.
- Resolve behavior regressions and blocking findings before merging. Open P0/P1 findings block merging; fix P2 findings or record acceptance and follow-up.
- Keep affected English and Chinese docs accurate. Mark milestones complete only when their acceptance criteria are verified.
- When the user already authorized merging, merge a qualifying PR without asking again. Then obtain the actual merged commit SHA and verify the main push CI for that exact SHA; all required jobs must pass before declaring completion, publishing, or cleaning up diagnostic worktrees. If main advances again, check its latest gates too.
- If post-merge CI fails, pause subsequent feature merges/releases, preserve evidence, identify the cause, and fix or assess a revert before verifying the restored main. Missing/cancelled/skipped required checks are not success. Follow separate release/deployment instructions for those actions.
- Preserve needed artifacts, archive completed managed worktrees with the app tool, and verify the archive record, directory removal, and Git worktree list. Synchronize only an idle, clean checkout.
- These instructions do not configure GitHub branch protection; do not claim remote enforcement without checking it.
