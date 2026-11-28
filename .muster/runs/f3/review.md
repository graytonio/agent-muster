VERDICT: APPROVE

## Issues

None.

## Notes

- Reviewed commit 3375b29 against the brief and plan. The local `main` ref is stale (845bb03), so `git diff main...HEAD` also shows the already-merged f2 (`--brief`) changes. Those are not part of this change.
- Checks I ran myself: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` all pass, including the three new `resume_detach_*` tests.
- `src/engine.rs`: `resume_preflight` keeps the old order: id, existence, lock, load, then the `Done` short-circuit before the cleaned and concurrency checks. Foreground behavior is unchanged. `drive` is untouched. The foreground path checks concurrency twice, once in the preflight and again under the start lock that is held while the state changes. The second check is the one that keeps the check and the reservation atomic, so the first one is cheap redundancy.
- `src/main.rs`: `start --detach` and `resume --detach` share `spawn_detached`. The lock is dropped before the child is spawned. A `done` run is reported synchronously. The TOCTOU window between the parent's preflight and the child's lock is documented in `docs/DECISIONS.md`.
- Optional: a test for `resume --detach` hitting the concurrency limit (exit 5) would cover the last synchronous error path the plan lists. It goes through the same `check_concurrency` helper as the foreground path, so the risk is low.
