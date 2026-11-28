VERDICT: APPROVE

## Issues

None.

## Notes

Checks re-run by the reviewer: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` all pass (24 engine tests, including the 5 new ones).

The change meets the brief and the plan. The new git calls all live in `src/git.rs` (`fetch_branch`, `is_ancestor`, `merge_in_progress`, `merge`). The fetch uses `st.remote`/`st.base_branch`, which already include the per-repo overrides. The merge is `--no-ff` with the requested message. A conflict is aborted (`merge --abort`) and escalates with the conflicting files and the resume steps. Finish refuses to run while a merge is in progress. Nothing is force-pushed. Resume reruns only finish, and the tests cover it. The docs are updated in both USAGE.md sections, and DECISIONS.md has the merge-not-rebase line.

Optional improvements (none block the merge):

1. `tests/engine.rs`, `finish_conflict_escalates_without_pushing`: `head_before` is read *after* `conflict_setup` has already escalated. That makes `assert_eq!(..., head_before)` compare HEAD with itself. The real "branch unchanged" guarantee comes from the `log --merges` being empty and the clean status/no-MERGE_HEAD checks. That's enough, but the `head_before` assertion is misleading. To make it meaningful, record HEAD before the resume (split `run_with_moved_base`), or drop it.
2. `src/git.rs` `merge`: if the merge fails without conflicts (no unmerged paths), the error message puts stderr and stdout together with a single space. It's fine, but `{stderr}\n{stdout}` or just the non-empty one would read better.
3. `src/engine.rs` `merge_base`: when `is_ancestor` fails, the reason says "merge ... failed". Something like "checking whether the branch contains {base} failed" would point more clearly at the cause.
4. When `open_pr = false`, finish skips the merge. This is recorded as an assumption in the plan's Risks. Consider adding a DECISIONS.md line for it.
