# Plan: merge the base branch into the run branch before opening the PR

## Goal

Fixes #9

Before muster pushes a run branch at finish, it fetches `<remote>/<base_branch>` and merges it into the run branch. That way the PR it opens never conflicts with the base, so GitHub runs `pull_request` checks on it. On a conflict the run escalates and names the conflicting files. A human merges by hand, then runs `muster resume`.

## Summary

`Engine::finish` (`src/engine.rs`) gets a new step before `git push`. It fetches the run's base, checks whether the run branch already contains it, and if not, makes a merge commit (`--no-ff`, never a rebase). A conflicting merge is aborted and escalates as `needs_human` with the conflicting files listed. `resume` reruns only the finish step, as it already does for other finish failures. New git helpers go in `src/git.rs`. Four engine integration tests cover the up-to-date, clean-merge, conflict and resume-after-resolution cases. `docs/USAGE.md` and `docs/DECISIONS.md` are updated.

## Steps

1. **Git helpers in `src/git.rs`** (no behaviour change yet). Add these methods to `impl Git`:
   - `fetch_branch(&self, dir, remote, branch) -> Result<()>` runs `git fetch --quiet <remote> +refs/heads/<branch>:refs/remotes/<remote>/<branch>`. The explicit refspec updates the tracking ref even if the repo's fetch refspec is unusual.
   - `is_ancestor(&self, dir, ancestor, descendant) -> Result<bool>` runs `git merge-base --is-ancestor <a> <d>` through `exec`. Exit 0 returns `Ok(true)`, exit 1 returns `Ok(false)`, and any other exit is an error that includes stderr.
   - `merge_in_progress(&self, dir) -> bool` reports whether `git rev-parse -q --verify MERGE_HEAD` succeeds (same pattern as `branch_exists`).
   - `merge(&self, dir, rev, message) -> Result<MergeOutcome>` with `pub enum MergeOutcome { Merged, Conflict(Vec<String>) }`. It runs `git merge --no-ff --no-edit --no-verify -m <message> <rev>` through `exec`. On success it returns `Merged`. On failure it collects the conflicting paths with `git diff --name-only --diff-filter=U` and then runs `git merge --abort`. If an abort fails, that is an error. If the paths list is non-empty it returns `Conflict(paths)`. If it is empty (the merge failed for another reason), it bails with the merge's stderr. **Every exit path leaves no `MERGE_HEAD` and the branch tip unchanged on a conflict.**
   - Optionally, add a small `#[cfg(test)]` unit test in `src/git.rs` or rely on the integration tests in step 3. Integration tests are enough.

2. **Finish step in `src/engine.rs`** (`Engine::finish`):
   - At the very start of `finish` (before the `is_dirty`/leftover-commit block), if `self.git.merge_in_progress(&wt)`, escalate with `finish: a merge is in progress in <wt>; finish it (resolve and commit) or run git merge --abort, then run: muster resume <id>`. Without this, `commit_all` could commit a half-resolved merge with conflict markers, and we must never push a branch with an in-progress merge.
   - Inside `if finish.open_pr { ... }`, before `self.git.push(...)`, call a new private method `fn merge_base(&self, st: &mut RunState, wt: &Path) -> Result<Option<String>>`. It returns `Some(reason)` when the run must escalate:
     - `let base = format!("{}/{}", st.remote, st.base_branch);` Use `st.remote` and `st.base_branch`: they were resolved at start with per-repo overrides (`Config::base_branch_for`).
     - `fetch_branch(wt, &st.remote, &st.base_branch)`. On error, return the reason `finish: git fetch <base> failed: {e:#}` (also logged as an `error` event, like the existing `step` closure does).
     - If `is_ancestor(wt, &base, "HEAD")` is true, return `None` and do nothing else. No event or commit.
     - Otherwise call `merge(wt, &base, &format!("muster: merge {base} into {} for {}", st.branch, st.id))`.
       - On `Merged`, record the event `st.event("merged", None, format!("{base} into {} ({})", st.branch, short head sha), None)`, `self.save(st)?`, and return `None`.
       - On `Conflict(files)`, record an `error` event and return the reason `finish: merging {base} into {branch} conflicts in {files.join(", ")}; merge it by hand in {wt} (git merge {base}, resolve, commit), then run: muster resume {id}`.
       - An `Err` from merge (including a failed abort) also becomes a `finish: merge {base} failed: {e:#}` reason.
     - In `finish`, on `Some(reason)` do `return self.escalate(st, "finish", reason)`. Push and PR creation then follow unchanged. Never pass `--force` to push (`Git::push` already does not).
   - Resume needs no change. A finish escalation leaves `cursor.stage == None`, so `drive` goes straight to `finish` and the merge is retried. After a human merge, `is_ancestor` is true and the merge is a no-op.

3. **Tests in `tests/engine.rs`**, alongside `dirty_worktree_gets_leftover_commit`, using `Env`, fake-herdr and the bare remote at `<root>/remotes/sandbox.git`:
   - Helper (in `tests/common/mod.rs` or on `Env`): `advance_base(root, file, content) -> String`. It clones `<root>/remotes/sandbox.git` into a temp dir under `root`, sets user.name/email, writes and commits `file`, pushes to `main`, and returns the new commit sha.
   - To move the base *during* a run, use the existing timeout-then-resume pattern. The scenario has `"planner": [{"error": "timeout"}]`. After `start` escalates (exit 3), call `advance_base(...)`, write `plan.md` by hand (as in `timeout_escalates_and_resume_accepts_hand_finished_stage`), then `muster resume f1`.
   - `finish_merges_moved_base`: the base change touches `BASE.md` and the builder commits `src/lib.rs` (no overlap). Resume exits 0 with status `done` and `pr_url` set. In the bare remote, `git merge-base --is-ancestor <base sha> muster/feat/f1` succeeds. The worktree `git log --merges --format=%s` contains `muster: merge origin/main into muster/feat/f1 for f1`. The state's events include a `merged` event.
   - `finish_conflict_escalates_without_pushing`: the base changes `README.md` and the builder `commit_repo`s a different `README.md`. Resume exits 3 with status `needs_human`, and the reason contains `README.md` and `muster resume f1`. In the worktree: no `MERGE_HEAD` (`git rev-parse -q --verify MERGE_HEAD` fails), `git status --porcelain` is empty, and `git log --merges` is empty (HEAD is unchanged). The bare remote has no `muster/feat/f1` branch, and `<root>/gh.args` does not exist.
   - `finish_resume_after_manual_merge_opens_pr`: same setup as the conflict test (it can share a setup fn). After the escalation, resolve by hand in the worktree with `git merge -X ours --no-edit origin/main` (via `common::git`, which sets an identity). Then `muster resume f1` exits 0 with status `done`, `pr_url` set and `gh.args` present. The remote branch contains the base sha, and only the human's merge commit exists (`git log --merges` has exactly one entry, and it is not the muster merge message).
   - `finish_without_base_change_makes_no_merge`: `happy_path_opens_pr` already covers the flow, so either add assertions there or add a short test. `git log --merges` in the worktree is empty and no `merged` event exists.
   - Optional: a test that a leftover in-progress merge escalates (start a conflicting `git merge` by hand in the worktree before resume and assert the "merge is in progress" reason). Nice to have.

4. **Docs**:
   - `docs/USAGE.md`, "What a run does": add a step saying that before pushing, finish fetches `<remote>/<base_branch>`. If the base has moved, it merges the base into the run branch with a merge commit (`muster: merge … for <run>`). If the branch already contains the base, nothing happens.
   - `docs/USAGE.md`, "Handling an escalated run": add the merge-conflict case. The reason names the conflicting files. Run `git merge <remote>/<base_branch>` in the worktree, resolve, commit, then `muster resume <run>`. Only finish reruns, and the merge retry is a no-op. Also mention the in-progress-merge escalation.
   - `docs/DECISIONS.md`: add one line under an appropriate section (e.g. a new `## Finish` or near the engine notes). For example: "Finish merges `<remote>/<base_branch>` into the run branch (merge commit, `--no-ff`) rather than rebasing: the branch history is the paper trail, a rebase would rewrite commits already pushed and require a force-push, and GitHub won't run `pull_request` checks on a conflicting PR."

5. **Verify**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.

## Definition of done

- [ ] `src/git.rs` has `fetch_branch`, `is_ancestor`, `merge_in_progress` and `merge` (returning `MergeOutcome`), and no git calls were added anywhere else.
- [ ] `Engine::finish` fetches and merges the base before `push` when `open_pr` is set, records a `merged` event on a merge, and makes no merge commit when the branch already contains the base.
- [ ] On a conflict, the merge is aborted (no `MERGE_HEAD`, branch tip unchanged, clean worktree) and the run is `needs_human` with a reason that lists the conflicting files and says to merge by hand, commit, then `muster resume <run>`. Nothing is pushed and no PR is created.
- [ ] `muster resume` after a manual merge reruns only finish, then pushes and opens the PR.
- [ ] Finish refuses to proceed (escalates) when a merge is in progress in the worktree. No force-push anywhere.
- [ ] The four new tests in `tests/engine.rs` (clean merge, conflict, resume after manual merge, base unchanged) pass, and the existing tests still pass.
- [ ] `docs/USAGE.md` ("What a run does", "Handling an escalated run") and `docs/DECISIONS.md` (one line: merge, not rebase, and why) are updated.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy --all-targets -- -D warnings` passes.
- [ ] `cargo test` passes. (`just` is not installed; run cargo directly.)

## Risks

- **Base merge only when `open_pr` is set.** The brief says "before pushing", and muster only pushes when `finish.open_pr` is true. Assumption: with `open_pr = false`, finish does not fetch or merge, since there is no PR to conflict with.
- **Fetch scope.** The brief asks to fetch `<remote>/<base_branch>`. Assumption: fetch only that branch with an explicit refspec into `refs/remotes/<remote>/<base_branch>`, not the whole remote as `setup` does. Worktrees share refs with the main repo, so running it in the worktree is fine.
- **`--no-ff`.** The run branch always has at least the `muster: start` commit, so a fast-forward can't happen in practice. `--no-ff` guarantees the "merge commit, not rebase" requirement anyway.
- **`--no-verify` on merge.** This matches `commit_all`/`commit_path`, which skip hooks. Assumption: repo hooks should not block muster's own bookkeeping commits.
- **In-progress merge on resume.** The brief doesn't mention it. A human who starts the merge but doesn't commit before `resume` would otherwise hit the leftover-commit path, and `git add --all && git commit` would commit conflict markers. Assumption: escalate instead, as "never push a branch with an in-progress merge" requires.
- **Moving the base mid-run in tests.** fake-herdr has no "run a command" action. Assumption: the tests use the existing escalate-then-resume flow (planner timeout, then move the base, then resume) instead of extending fake-herdr.
- **Event kind name.** The brief says "record an event" without naming it. Assumption: kind `merged`, with the message `<remote>/<base> into <branch> (<sha>)`. Tests match on the kind.
- **`Fixes #9`.** The brief asks for it at the top of `## Goal`. The prompt's required sections don't include a Goal, so a `## Goal` section was added first. `plan.md` is in `feature.toml`'s `pr_body_from`, so the line lands in the PR body.
