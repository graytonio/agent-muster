## Changes
- `src/engine.rs`: new `check_concurrency` helper (used by `reserve` and `resume`) and `Engine::resume_preflight`; `resume` now calls the preflight. `drive` untouched.
- `src/main.rs`: `detach()` split into `detach_start` and shared `spawn_detached`; `cmd_resume` takes `detach` and runs the preflight, releases the lock, then spawns `muster resume <run>` (a `done` run is reported synchronously, nothing spawned).
- `src/cli.rs`: `--detach` on `resume`.
- `tests/engine.rs`: `resume_detach_runs_in_background`, `resume_detach_unknown_run_fails_synchronously`, `resume_detach_locked_run_fails_synchronously`.
- `docs/USAGE.md`, `docs/RUNTIME.md`, `docs/DECISIONS.md`: updated as planned.

## Checks
- `cargo fmt --check`: pass
- `cargo clippy --all-targets -- -D warnings`: pass
- `cargo test`: pass (all suites, including the three new tests)

## Deviations
- In `resume_preflight` the cleaned-run refusal applies only when the status is not `Done`, so a cleaned `done` run still reports "already done" as before (keeps foreground behavior unchanged).
