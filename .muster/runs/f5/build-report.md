## Changes

- `src/git.rs`: added `MergeOutcome`, `fetch_branch`, `is_ancestor`, `merge_in_progress` and `merge` (aborts on conflict, returns the conflicting paths).
- `src/engine.rs`: `Engine::finish` escalates if a merge is in progress. Before `push` (when `open_pr` is set), the new `merge_base` fetches `<remote>/<base_branch>`, does nothing if the branch already contains it, and otherwise makes a `--no-ff` merge commit and records a `merged` event. A conflict, or a fetch or merge failure, escalates with a reason naming the files and the `muster resume` instructions.
- `tests/common/mod.rs`: `advance_base` helper.
- `tests/engine.rs`: `finish_merges_moved_base`, `finish_conflict_escalates_without_pushing`, `finish_resume_after_manual_merge_opens_pr`, `finish_without_base_change_makes_no_merge`, plus the optional `finish_with_merge_in_progress_escalates`.
- `docs/USAGE.md`, `docs/DECISIONS.md`: documented the extra finish step, the merge-conflict case and the merge-not-rebase decision.

## Checks

- `cargo fmt --check`: pass
- `cargo clippy --all-targets -- -D warnings`: pass
- `cargo test`: pass (all suites, including the 5 new tests)

## Deviations

None.
