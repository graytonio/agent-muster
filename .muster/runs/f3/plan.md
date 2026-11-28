# Plan: `--detach` for `muster resume`

## Goal

Fixes #7

`muster resume <run> --detach` checks up front that the run can be resumed, prints the run id (or JSON with `--json`), and returns immediately. A background `muster resume <run>` then drives the run and logs to `<log_dir>/<run>.log`, just like `start --detach`.

## Summary

Add a `--detach` flag to `resume` in `src/cli.rs`. Pull the spawn-and-log half of `detach()` in `src/main.rs` out into a shared helper, so `start --detach` and `resume --detach` re-exec through the same code. Move the up-front part of `Engine::resume` into an `Engine::resume_preflight` that both the foreground resume and the detaching parent call. Unknown, cleaned, locked or over-the-limit runs then fail synchronously with the same exit codes as a foreground resume. The stage logic (`drive` and everything after it) does not change.

## Steps

1. **Factor out the resume preflight in `src/engine.rs` (behavior unchanged).**
   - Add a private helper `fn check_concurrency(&self, except: Option<&str>) -> Result<()>`. It holds the existing `store.active(except)` check and the `MusterError::new(exit::CONCURRENCY, "concurrency limit reached: …")` message, and it does not take the start lock itself (callers hold `store.start_lock()`). Use it in both `Engine::reserve` (with its current `except` argument) and `Engine::resume`, so the message lives in one place.
   - Add `pub fn resume_preflight(&self, id: &str) -> Result<(RunLock, RunState)>`. It does, in today's order: `check_run_id(id)`, the `store.exists(id)` check (usage error "no run named {id}", exit 2), `store.lock(id)` (usage error "being driven by another muster process", exit 2), `store.load(id)`, the `cleaned_at` refusal (exit 2), and, only when `st.status != Done`, a concurrency check under `store.start_lock()` (exit 5) that changes nothing. It returns the held lock and the loaded state. A `Done` run returns `Ok` so the caller can print "already done", as it does today.
   - Rewrite the top of `Engine::resume` to call `resume_preflight`, then keep the existing `Done` early return and the existing block under `start_lock` that re-checks concurrency, sets `Running`/`pid`, writes the `resumed` event and saves. Keep the snapshot re-validation, the `worktree_path.is_none()` failure and `drive` unchanged. The second concurrency check stays under the same start lock as the state change, so the check and the reservation stay atomic in the foreground path.
   - `RunLock` must be importable from `muster::state` (it is already `pub`).
   - Existing tests must still pass unchanged (`cargo test`).

2. **Share the re-exec code in `src/main.rs`.**
   - Split `detach()` into:
     - `detach_start(g, cfg, args: StartArgs) -> Result<i32>`: today's id selection, `check_run_id`, the "already exists" check and the brief-file handoff. It builds the child argv (`start <wf> --repo <repo> --run-id <id> --brief-file <path> [--skip s]...`) and calls the helper below.
     - `spawn_detached(g: &GlobalArgs, cfg: &Config, id: &str, child_args: &[OsString]) -> Result<i32>`: `create_dir_all(log_dir)`, opens `<log_dir>/<id>.log` in append mode, builds `current_exe()` with `--config-dir <cfg.config_dir>` and `-v` if verbose, adds `child_args`, sets stdin to null, sends stdout/stderr to the log, calls `process_group(0)` and spawns. It then emits `{"ok": true, "run_id", "pid", "log"}` (JSON) or prints the id (human), and returns `exit::OK`. The code is moved from the current `detach()` unchanged.
   - `cmd_start` calls `detach_start`. Behavior and output stay byte-for-byte the same; the existing `detach_prints_run_id_and_runs_in_background` and `inline_brief_detach` tests cover this.

3. **Add the CLI flag in `src/cli.rs`.**
   - Change `Command::Resume { run_id: String }` to also take `#[arg(long)] detach: bool`, with the doc comment `/// Run in the background, logging to <log_dir>/<run>.log, and print the run id` (same wording as `StartArgs::detach`).
   - In `src/main.rs` `run()`: `Command::Resume { run_id, detach } => cmd_resume(g, &run_id, detach)`.

4. **Implement `resume --detach` in `src/main.rs` `cmd_resume`.**
   - When `detach` is false, keep the current path.
   - When `detach` is true:
     1. Build `real_herdr` and `Engine::new(&cfg, &herdr, g.verbose, !g.json)` as today; the preflight makes no herdr calls.
     2. `let (lock, st) = engine.resume_preflight(id)?;`. Any error returns through `main` with its usual exit code (2 for unknown, cleaned or locked runs, 5 for the concurrency limit) and the usual `{"ok": false, …}` JSON. Nothing is spawned and no log file is created.
     3. If `st.status == RunStatus::Done`, print the outcome the way a foreground resume does and return 0 without spawning: `drop(lock)`, then `return Ok(print_outcome(g, &engine.resume(id)?))`. `Engine::resume` already handles `Done` by printing "already done" and returning the outcome without driving anything, so `Outcome::from_state` can stay private.
     4. `drop(lock)` and then call `spawn_detached(g, &cfg, id, &["resume".into(), id.into()])`. The lock must be released before the child starts, because the child acquires it in its own `Engine::resume`.
   - Like the start child, the child gets no `--json`, so its log holds human output and progress lines, appended after the `start` output in the same `<run>.log`.

5. **Tests in `tests/engine.rs`** (use `Env`, `fake-herdr` and the same bounded polling loop as `detach_prints_run_id_and_runs_in_background`):
   - `resume_detach_runs_in_background`: scenario `planner: [{}, {"write": {"plan.md": "# Plan\n"}}]`, builder and reviewer approving (as in `missing_artifact_escalates_then_resume_reprompts`). `e.start("feature", &[])` exits 3. Then `e.muster(&["--json", "resume", "f1", "--detach"])`. Assert exit 0, `v["ok"] == true`, `v["run_id"] == "f1"` and that `v["log"]` ends with `f1.log`. Assert that it returned promptly: measure with `std::time::Instant` around the call and allow a generous bound such as under 10 s, or rely on the polling after it. Poll `e.state("f1")["status"]` until `"done"` (at most 200 × 50 ms) and show the log contents in the assert message. Also assert the planner was prompted twice (`e.herdr_calls(&["agent","prompt","planner-f1"]).len() == 2`).
   - `resume_detach_unknown_run_fails_synchronously`: `e.muster(&["resume", "f9", "--detach"])` exits with `Some(2)`, the same code as the foreground `resume f9` in `usage_errors`. Assert that `e.root.join("logs/f9.log")` does not exist, which proves nothing detached. Also run `--json resume f9 --detach` and assert `v["ok"] == false` and `v["exit_code"] == 2`.
   - Optional, but cheap and worth having: `resume_detach_locked_run_fails_synchronously`. Escalate a run (planner `[{}]`), hold `state/f1.lock` with `try_lock()` as `clean_refuses_running_run_without_force` does, and assert `resume f1 --detach` exits 2 with "another muster process" in its output and no extra content in `logs/f1.log` (it must not exist, since the run was started in the foreground).

6. **Docs.**
   - `docs/USAGE.md` line 32: the `resume <run>` row becomes `` `resume <run> [--detach]` `` with the text "Continue a run from its recorded stage, using its config snapshot. `--detach` checks the run can be resumed, prints its id and continues in the background, logging to `<log_dir>/<run>.log`."
   - `docs/USAGE.md` escalation section (the lines around 56–69 that say `muster resume <run>`): add a sentence such as "Over SSH, use `muster resume <run> --detach` so the session doesn't stay open until the run finishes; follow it with `muster status <run>` or the log in `<log_dir>/<run>.log`." Optionally add the `ssh muster 'muster resume f1 --detach'` form next to the existing line-21 SSH examples.
   - `docs/RUNTIME.md` line 39 (`log_dir`: "Logs of `--detach` runs") is still accurate. Line 49 mentions only `muster start --detach`: change it to "`muster start --detach` and `muster resume --detach` re-execute…".
   - `docs/DECISIONS.md`: after the existing `--detach` line (about line 53), add one line: "`resume --detach` runs `Engine::resume_preflight` in the parent (run id, existence, lock, cleaned, concurrency; same exit codes as foreground), releases the lock, then re-execs `muster resume <run>` through the same spawn helper as `start --detach`, appending to `<log_dir>/<run>.log`. A process that grabs the lock or the last concurrency slot between the parent's check and the child's start makes the child fail, and the error lands in the log. A run already `done` is reported synchronously and nothing is spawned."

7. **Verify:** run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` (`just` isn't installed).

## Definition of done

- [ ] `muster resume <run> --detach` is accepted: `--detach` is no longer an "unexpected argument".
- [ ] `muster --json resume f1 --detach` on a `needs_human` run prints `{"ok":true,"run_id":"f1","pid":…,"log":"…/f1.log"}`, exits 0 right away, and the run reaches `done` in the background.
- [ ] Without `--json`, `resume --detach` prints just the run id, as `start --detach` does.
- [ ] `muster resume f9 --detach` on an unknown run exits 2 (the same code as `muster resume f9`), spawns nothing and creates no `f9.log`.
- [ ] A cleaned run (exit 2), a locked run (exit 2) and the concurrency limit (exit 5) are reported synchronously by `resume --detach`, with the same codes as a foreground resume.
- [ ] `start --detach` and `resume --detach` share one spawn helper in `src/main.rs`, with no duplicated `Command` building or log opening.
- [ ] Foreground `resume` behaves as before, and every existing test in `tests/` passes unchanged.
- [ ] `drive` and the per-stage logic in `src/engine.rs` are untouched; only the resume preamble is refactored into `resume_preflight`, plus the `check_concurrency` helper.
- [ ] New tests `resume_detach_runs_in_background` and `resume_detach_unknown_run_fails_synchronously` (plus the optional locked-run test) are in `tests/engine.rs` and pass.
- [ ] `docs/USAGE.md` has an updated `resume` row and escalation section, `docs/RUNTIME.md` line 49 mentions resume, and `docs/DECISIONS.md` has one new line.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy --all-targets -- -D warnings` passes.
- [ ] `cargo test` passes.

## Risks

- **Time-of-check to time-of-use.** The parent checks the lock and the concurrency limit, then releases the lock so the child can take it. Another process can take the run lock or the last concurrency slot in that window, and the child then fails with the usual error in `<run>.log`. Assumption: this is acceptable and matches the documented race for `start --detach` (DECISIONS line 53). Handing the flock to the child through an inherited fd would close the window, but the child's own `store.lock()` opens a new file description and would conflict with it. That needs more engine plumbing than the brief allows.
- **Which checks run synchronously.** The brief lists an unknown run, a non-resumable run and a locked run. Assumption: "non-resumable" means a cleaned run, and the concurrency limit is also checked up front ("concurrency limit behaves exactly as for a foreground resume"). The `worktree_path.is_none()` failure (exit 4) is left where it is, after the state is marked running and inside the child, so foreground behavior and engine ordering don't change. It is a rare case: a start that failed before its worktree existed.
- **An already-`done` run.** A foreground resume prints "already done" and exits 0. Assumption: `resume --detach` does the same synchronously and spawns nothing. Spawning a child just to exit at once would be pointless.
- **The child's output.** As with `start --detach`, the child runs without `--json`, so the log holds human output. `--verbose` is forwarded and `--config-dir` is always passed explicitly, both as `spawn_detached` already does. The resume output is appended to the log file the start wrote.
- **"Returns promptly" in the test.** Wall-clock assertions are flaky on CI. Assumption: a generous bound (for example under 10 s) plus the background completion check is enough. The real proof is that the parent returns before the background run's final state is reached.
- **Engine change vs. "no changes to the engine's stage logic".** Extracting `resume_preflight` and `check_concurrency` is a refactor of the preamble, not of stage logic. Assumption: this is allowed, and it is better than duplicating the checks in `main.rs`, which the brief also forbids in spirit ("reuse it rather than duplicating").
- **Plan sections.** The brief asks for a `## Goal` section starting with `Fixes #7`, and the planner prompt requires Summary, Steps, Definition of done and Risks. The plan includes both.
