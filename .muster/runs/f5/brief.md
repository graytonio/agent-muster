Merge the base branch into the run branch before opening the PR, and escalate on conflicts

Fixes #9 (https://github.com/graytonio/agent-muster/issues/9). Put the line `Fixes #9` at the top of the plan's `## Goal` section so it ends up in the PR body.

## Problem

A run branches from `<remote>/<base_branch>` when it starts. If the base moves while the run is in progress and touches the same files, the PR muster opens at finish conflicts with the base. GitHub doesn't run `pull_request` workflows on a conflicting PR, so the PR shows no checks at all, and a required status check blocks it with no hint why.

This happened with run f2 (PR #6): `docs/DECISIONS.md` gets appended to by almost every change, so it will keep happening.

## Requirements

In the finish step, before pushing (see the finish logic in `src/engine.rs`, and route every git call through `src/git.rs` as the conventions in `CLAUDE.md` require):
- Fetch `<remote>/<base_branch>`, using the run's effective remote and base branch, including per-repo overrides.
- If the run branch already contains the base, do nothing extra.
- Otherwise merge the base into the run branch with a merge commit, not a rebase: the branch history is the paper trail. Use a clear message such as `muster: merge <remote>/<base_branch> into <branch> for <run>`, record an event, and continue to push and open the PR as today.
- If the merge conflicts:
  - run `git merge --abort` so the worktree and branch are exactly as before
  - escalate (`needs_human`), with a reason that names the conflicting files and says how to resolve: merge the base in the worktree by hand, commit, then `muster resume <run>`
  - on `resume`, only the finish step reruns. This matches the existing "finish failed" behaviour, and the merge is attempted again; after a human merge it's a no-op.
- Never push a branch with an in-progress merge, and never force-push.

## Tests

Add tests alongside the existing engine tests, with `fake-herdr` and a local bare remote, as the other finish tests do:
- The base moved without conflicts: the run finishes, the pushed branch contains the base commit, and a merge event is recorded.
- The base moved with a conflict: the run escalates with the conflicting file in the reason, the branch is unchanged, and nothing is pushed.
- After the conflict is resolved by hand and `resume` runs: the run finishes and opens the PR.
- The base did not move: no merge commit is created.

## Docs

`docs/USAGE.md`: describe the extra finish step in "What a run does", and add the merge-conflict case to "Handling an escalated run". Add one line to `docs/DECISIONS.md` (merge, not rebase, and why).

## Definition of done

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` all pass. The Rust toolchain is installed: run the cargo commands directly, since `just` isn't installed.
