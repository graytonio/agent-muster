# Plan: `muster start --brief <TEXT>`

## Summary

Add a `--brief <TEXT>` flag to `muster start` so a short brief can be passed inline instead of through a file or stdin. The three brief sources become one required, single-choice clap `ArgGroup`, so passing two of them, or none, is a clap usage error (exit 2). The engine already has `Brief::Text(String)` (`src/engine.rs:72`), currently unused, so the work is mostly CLI wiring in `src/cli.rs` and `src/main.rs` (`cmd_start` and `detach`), plus tests and docs.

## Steps

1. **CLI flag and group (`src/cli.rs`, `StartArgs`).**
   - Import `ArgGroup` from clap.
   - Put a group on `StartArgs`:
     `#[command(group(ArgGroup::new("brief_source").required(true).multiple(false).args(["brief", "brief_file", "brief_stdin"])))]`.
   - Add the field `pub brief: Option<String>` with `#[arg(long, value_name = "TEXT")]` and the doc comment `/// The brief text itself (its first line becomes the PR title)`. Put it before `brief_file`.
   - Remove `conflicts_with = "brief_stdin"` and `required_unless_present = "brief_stdin"` from `brief_file`, because the group replaces them. Keep `value_name = "PATH"`.
   - Check by hand with `cargo run -- start x --repo y --brief a --brief-stdin` and with no brief flag at all: both must exit 2 with clap's "cannot be used with" / "the following required arguments were not provided" message.

2. **One place that maps args to a `Brief` (`src/main.rs`).**
   - Add `fn brief_source(args: &StartArgs) -> Brief` that returns `Brief::Text(t.clone())` for `args.brief`, `Brief::File(p.clone())` for `args.brief_file`, and `Brief::Stdin` otherwise.
   - In `cmd_start`, replace the inline `match &args.brief_file { ... }` with `brief_source(&args).read()?`. `Brief::read` already rejects empty or whitespace-only text with `MusterError::usage("the brief is empty")` (exit 2), so an empty `--brief` is rejected the same way as an empty file or empty stdin. No change to `src/engine.rs` is needed.

3. **Detach support (`src/main.rs`, `detach`).**
   - Today `detach` canonicalizes `--brief-file` and passes it on. For stdin, it reads the brief, writes it to `<log_dir>/<id>.brief.md`, and re-execs the child with `--brief-file <that path>`.
   - Change the `brief_path` match so that `--brief` takes the stdin path: if `args.brief_file` is `Some`, keep the canonicalize branch. Otherwise call `brief_source(&args).read()?`, which covers both `Text` and `Stdin`, write the result to `<log_dir>/<id>.brief.md`, and use that path.
   - The child keeps getting `--brief-file`, which also avoids long briefs on its command line and in `ps` output. An empty `--brief` fails in the parent with exit 2, before anything is spawned, just as empty stdin does today.

4. **Tests (`tests/engine.rs`).** Reuse `Env`, `happy_agents()`, `e.muster(...)`, `e.state`, `e.run_dir` and `out`.
   - `inline_brief_starts_run`: run `e.muster(&["start", "feature", "--repo", "sandbox", "--brief", "Fix the flaky retry test\n\nMore detail."])` with the happy scenario. Assert exit 0. Assert that `run_dir("f1")/brief.md` equals the text and is committed (`git ls-files .muster` contains `.muster/runs/f1/brief.md`). Assert that `gh.args` contains `Fix the flaky retry test` (the PR title) and does not contain `More detail.`. Assert that `muster status f1 --json` returns `ok: true` with `status: "done"` and a `run_dir`, so the run is visible in status.
   - `inline_brief_detach`: copy `detach_prints_run_id_and_runs_in_background`, but pass `--brief "Detached inline brief" --detach` and no stdin. Wait for `done` and assert that `brief.md` contains the text. Also assert that `logs/f1.brief.md` exists.
   - Brief-source usage errors: extend `usage_errors` or add `brief_source_conflicts_exit_2`. Each of these must exit 2:
     - `--brief x --brief-file <some existing file>`
     - `--brief x --brief-stdin`
     - `--brief-file f --brief-stdin` (a regression check on the old conflict)
     - no brief flag (already covered)
     - `--brief "   "` (empty)

     For the conflict cases, also assert that stderr mentions `cannot be used with`, so the test proves clap rejected the arguments and not a later check.

5. **Docs.**
   - `docs/USAGE.md`:
     - Line 12-13 quick start: add `muster start bugfix --repo muster-sandbox --brief "Fix the flaky retry test in src/net.rs"`.
     - Line 20 (SSH): add a one-shot example with `--brief` (quote it for the remote shell: `ssh muster 'muster start bugfix --repo foo --brief "Fix the flaky retry test" --detach'`).
     - Line 30 `start` row: change it to `start <workflow> --repo <repo> (--brief <text> \| --brief-file <path> \| --brief-stdin)` and say that exactly one is required.
   - `README.md`: in the "Use" block (around line 90), add a `--brief "..."` example. In the SSH block (around line 101), add the inline one-shot form.
   - `docs/DECISIONS.md`: add one line. The three brief sources are a required, non-multiple clap `ArgGroup` (`brief_source`). `--brief --detach` writes the text to `<log_dir>/<run>.brief.md` and re-execs with `--brief-file`, like stdin does, so the child's argv stays short. Also update the existing `--detach` line (line 53), which mentions only the stdin brief, to say "a stdin or inline brief".

6. **Verify.** Run `cargo fmt`, then `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` (or `just check`).

Steps 1 to 3 can be one commit (the feature), step 4 a second, and step 5 a third.

## Definition of done

- [ ] `muster start <wf> --repo <r> --brief "text"` starts a run. `.muster/runs/<run>/brief.md` contains exactly the text and is committed on the run branch.
- [ ] The PR title (`gh pr create` args) is the first line of the inline brief.
- [ ] `muster start ... --brief "text" --detach` prints the run id, the background run completes, and its `brief.md` contains the text.
- [ ] `muster status <run>` shows the run started with `--brief`, as it does for the other sources.
- [ ] `--brief` with `--brief-file`, `--brief` with `--brief-stdin`, `--brief-file` with `--brief-stdin`, and no brief flag each exit 2 with a clap usage message.
- [ ] `--brief ""` and `--brief "   "` exit 2 with "the brief is empty", the same as empty stdin or an empty file.
- [ ] `conflicts_with` and `required_unless_present` are gone from `brief_file`, replaced by one `ArgGroup`.
- [ ] Nothing changed in workflows, prompts, `defaults/` or the engine's stage logic.
- [ ] `docs/USAGE.md` (start row and quick start), `README.md` (usage and SSH examples) and `docs/DECISIONS.md` are updated.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy --all-targets -- -D warnings` passes.
- [ ] `cargo test` passes.

## Risks

- **"Visible in `muster status`".** `status` and `status <id>` never show the brief text or its source today, for any source. They only show `run_dir`, which contains `brief.md`. I assumed the requirement means a `--brief` run must show up in status like any other run, not that status should start printing the brief. So I plan no change to status output, only a test that the run appears there. If the brief should appear in status, that is a new feature for all sources and is out of scope.
- **Empty briefs.** Empty files and empty stdin are already rejected by `Brief::read` (exit 2, "the brief is empty"). `--brief` goes through the same function, so all three are rejected the same way, with no new validation code. clap accepts `--brief ""` as an empty string, which then hits that check. Either way the exit code is 2.
- **Briefs that start with `-`.** `--brief "-x"` is parsed by clap as a flag. I did not set `allow_hyphen_values`, because then a missing value (`--brief --detach`) would silently take the next flag as the brief. Users can write `--brief=-x`. Note this in the USAGE start row if it fits in one clause.
- **Detach handoff.** I chose to write the inline text to `<log_dir>/<id>.brief.md` and pass `--brief-file` to the child, instead of passing `--brief <text>` through. This matches the stdin path and keeps long or multi-line text out of argv. Behavior is the same either way.
- **clap groups with a `bool` flag.** `brief_stdin` is a `SetTrue` flag. In clap 4, an unset flag's default does not count as present for group conflict or requirement checks, so the group works as intended. The step 1 manual check and the step 4 tests confirm it.
- **Unused `Brief::Text`.** The engine's `Brief::Text` variant already exists and has no callers. I assumed it was meant for this feature and reused it.
