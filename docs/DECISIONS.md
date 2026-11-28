# Decisions

One line per non-obvious choice, with the reason.

## Project

- Crate lives at the repo root (not `muster/`), binary name `muster`. The repo is the project; a nested dir adds nothing.
- `fake-herdr` is a second binary target (`tests/fake-herdr/fake_herdr.rs`) so integration tests get `CARGO_BIN_EXE_fake-herdr`. Release recipes build `--bin muster` only.
- `rust-version = 1.89` because run locks use `std::fs::File::try_lock` (stable in 1.89), which avoids a locking crate.
- Added `sha2` for the config-snapshot content hash.

## Operator answers to the plan's open questions

- Reviewer is Claude (opus) by default, not Codex. Removes `codex` from the runtime contract's required list (still a valid `kind`).
- No `tutorial` stage in the default workflows. Optional stages and `--skip` are still supported.
- PRs open ready for review (not drafts). `[finish] draft = true` is available per workflow.

## herdr (checked against the installed binary, herdr 0.9.3; the plan cites docs v0.9.5)

- Leaf `--help` (e.g. `herdr agent start --help`) prints the top-level help in 0.9.3. Usage comes from running the group bare (`herdr agent`) and from `herdr api schema --json`.
- Server errors are JSON `{"error":{"code","message"},"id"}` on **stderr** with exit 1; CLI syntax errors are plain text with exit 2 (e.g. an unknown `--kind`). `map_error` reads both streams and maps exit 2 without JSON to `Usage`.
- Not-found codes are specific (`agent_not_found`, `pane_not_found`, `workspace_not_found`); any `*_not_found` maps to `NotFound`.
- A busy pane returns `agent_pane_busy`; mapped to its own kind `PaneBusy`.
- `status server --json` prints a bare object, not `{"result":..}`.
- `worktree create` accepts `--base <ref>`, so runs branch from `origin/<base_branch>` after a `git fetch`. It also accepts `--path`; muster passes `<worktrees_dir>/<repo>/<run>` when `worktrees_dir` is set, otherwise herdr's default (`~/.herdr/worktrees/<repo>/<branch-with-dashes>`). The real path is read from `.result.worktree.path`.
- `worktree create` also opens a workspace for the source checkout (`source_workspace_id`). muster never closes that one; `clean` removes only the run's workspace.
- `worktree remove` without `--force` removed a worktree with untracked files (`.muster/`) in 0.9.3. `clean` still passes `--force` only when asked.
- `agent prompt --wait` without `--until`: herdr's docs say `--wait` already waits for the first settled `idle`/`done`/`blocked` and not to repeat those with `--until`.
- Agent results: `agent start` → `.result.agent` (`agent_started`, plus `argv`); `agent prompt` → `.result.agent` (`agent_prompted`); `agent get` → `.result.agent`; `agent read` → `.result.read.text`. Settled = `agent_status` in `idle|done`.
- `notification show` returns `{"shown":false,"reason":"disabled"}` when notifications are off; muster treats that as success.
- `pane report-metadata` prints nothing on success.
- `integration status` prints text lines (`claude: current (v10) (...)`, `codex: not installed (...)`), not JSON. doctor parses them.
- herdr 0.9.3 agent kinds: pi claude codex gemini cursor devin agy cline omp mastracode opencode copilot kimi kiro droid amp grok hermes kilo qodercli qwen letta maki muse.
- Fixtures in `tests/fixtures/herdr/` were recorded against an isolated session (`herdr --session mdev server`), with local paths replaced by `/home/muster`. Agent responses (`agent start`/`prompt` success, `agent_not_ready`, `agent_prompt_stalled`, timeout) still need a live-agent recording: see "Manual run" in `docs/USAGE.md`. The timeout code `timeout` comes from herdr's agent skill text, not a recording. **VERIFY.**

## Engine (Phases 4–5)

- Run ids are `[a-z][a-z0-9]*`, at most 11 characters; roles at most 20. Agent names `<role>-<run>` then always fit herdr's 32-character limit.
- Prompts pass the **absolute** prompt path ("Read /abs/.muster/runs/f1/prompts/plan-1.md and follow it exactly."), and template variables `run_dir`, `brief_path`, `produces` and `previous_review` are absolute too, so agents don't depend on their cwd.
- Before prompting a stage, an existing artifact from an earlier iteration is renamed to the first free `<stem>-<k>.<ext>` (e.g. `review-1.md`), so a stale file can't satisfy the new iteration. `previous_review` follows the rename.
- `cursor.prompted` is saved **before** `agent prompt` is sent. On `resume`, a prompted stage's artifact is re-checked first: a valid artifact (finished by hand) completes the stage without prompting. A missing artifact or a `## Blocked` artifact is the only case where resume prompts the stage again; that's an explicit human action, not an automatic retry.
- A human can leave `.muster/runs/<run>/human-notes.md` before `resume`; the re-prompt tells the agent to read it. This gives the human a way to answer a `## Blocked` question without editing the snapshot.
- `## Blocked` is detected as a line that is exactly `## Blocked` (case-insensitive) or starts with `## Blocked `; `### Blocked` and `## Blockers` don't count.
- A verdict stage's artifact is checked for `## Blocked` before the verdict line, so a reviewer can block below `VERDICT: CHANGES`.
- Stages can set `read_only = true` (planner, reviewer, investigator in the defaults). After the stage, muster lists files changed since the stage's start HEAD (committed or not, plus untracked) outside the run dir and escalates if there are any. Paths already dirty when the stage started are excluded, so a read-only stage isn't blamed for an earlier stage's leftovers. This is the enforcement for "investigate must not modify files"; Claude Code's own `--disallowedTools` can't scope writes to one directory.
- Agent result `agent_status` other than `idle`/`done` after `--wait` (i.e. `blocked`, `unknown`) escalates. `agent_blocked`, `agent_not_ready`, `agent_prompt_stalled`, `timeout` and every other herdr error escalate too. Nothing is retried.
- Panes: the first agent uses the worktree's root pane; each later role gets a new pane split from the most recent one, alternating right and down. On resume, a role whose agent vanished is restarted in its own pane if that pane still exists; if the pane is busy or gone, a new one is split.
- Agents are started with `--timeout 60000` (herdr's default is 30 s; a cold Claude start can be slower).
- muster's own commits (`muster: start f1`, `muster: plan iteration 1 for f1`) commit only the run dir (`git commit -- .muster/runs/<run>`), so they never sweep up an agent's uncommitted code. They use `--no-verify` so a target repo's pre-commit hooks can't fail on Markdown artifacts.
- Concurrency: a run counts as active only if its status is `running` **and** its lock file is held (flock). A crashed process leaves `running` in the state file but releases the lock; `status` shows such runs as `running (stale)` and `resume` picks them up. Run creation and the count are serialized with `<state_dir>/.start.lock`.
- `resume` re-checks the concurrency limit (excluding itself) and refuses runs that were cleaned.
- Exit codes: setup failures before the worktree exists (fetch, `worktree create`) mark the run `failed` (exit 4). Failures in the finish step (push, `gh pr create`) escalate (`needs_human`, exit 3) so `resume` can retry the finish once the cause is fixed. `pr_url` is recorded so a resumed finish never opens a second PR.
- `--detach` picks the run id in the parent (next free id), writes a stdin or inline (`--brief`) brief to `<log_dir>/<run>.brief.md`, and re-execs `muster start ... --run-id <id> --brief-file <path>` in a new process group with output to `<log_dir>/<run>.log`. Two detached starts racing for the same id: the second fails with "run already exists" in its log.
- The PR body is written to `<state_dir>/<run>/pr-body.md` (not committed): each `pr_body_from` file under a `## <file>` heading, then a footer with the run id, workflow and run dir.
- `clean` closes the run's agent panes, removes the worktree (`--force` only when given), optionally deletes the local branch, and keeps the state file with `cleaned_at` set so history survives.

## Entire (checked against docs.entire.com CLI reference; not installed locally)

- Setup per worktree: `entire enable --yes --agent <a>` once per entry in `[entire] agents` (default `["claude-code"]`), then `entire configure --local --checkpoint-remote PROVIDER:OWNER/REPO` if set. Failures are logged as warnings; the run continues.
- Finish logs `entire checkpoint list --outgoing --json --no-pager` into the event log. The docs say `--outgoing` works with the `git-refs` backend only.
- `ENTIRE_BIN` overrides the binary (used by tests to make it absent).

## Packaging (Phase 7)

- `just release` cross-builds static Linux binaries with `cargo zigbuild` (musl targets need a C toolchain for the target; zig provides one on any host). `just release-setup` installs the targets and cargo-zigbuild. `just release-host` builds only the host binary.
- `doctor` checks only the agent CLIs for kinds the config's workflows actually use, and the matching herdr integrations, so a Claude-only deployment doesn't need `codex`.
- `doctor` exits 4 (failed) when any check fails, 0 otherwise.

## CI

- One aggregate `ci` job (`needs: [lint, test, musl]`, `if: always()`) is the required check, so jobs can be added or renamed without touching branch protection.
- Tests run on ubuntu-latest (the deployment OS) and macos-latest (dev hosts). Both were checked locally: macOS natively, Linux in a `rust:1` container as a non-root user (the read-only config test needs a non-root user, which GitHub's runners are).
- The musl job builds x86_64 with `musl-tools` and asserts the binary is statically linked. That verifies the static Linux build in CI without cargo-zigbuild; aarch64 musl was checked once in an arm64 container.
- `--locked` everywhere so CI fails if `Cargo.lock` is out of date.
- Actions are pinned to major-version tags (`actions/checkout@v4`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`). Pin to commit SHAs if the org requires it.

## Toolchain and releases

- `rust-toolchain.toml` pins Rust 1.100.0 with rustfmt and clippy. CI was on floating `stable`, and the first run on `main` failed on a clippy lint new in 1.100 (`while_let_loop`). With the pin, toolchain bumps (and their new lints) happen in deliberate PRs. CI installs it with `rustup toolchain install`, which reads the file, instead of `dtolnay/rust-toolchain`.
- `Cargo.toml` keeps `rust-version = 1.89` as the minimum supported version; the pin is just what CI and contributors build with.
- Releases are tag-driven (`v*`). The workflow refuses a tag that doesn't match `Cargo.toml`'s version, calls `ci.yml` (`workflow_call`) so only commits passing the full suite ship, then builds natively on each runner: x86_64 musl on `ubuntu-latest`, aarch64 musl on `ubuntu-24.04-arm`, aarch64 macOS on `macos-latest`, and x86_64 macOS cross-compiled on `macos-latest` (not executed there). Native builds avoid needing zig/cargo-zigbuild in CI.
- The release workflow also runs (build only, no publish) on PRs that touch `release.yml`, the toolchain file or the Cargo manifests, so the matrix is exercised before a tag depends on it.
- Only the `publish` job gets `contents: write`; everything else is read-only.
- The server image is built in this repo (`container/`) from the release binaries rather than in the homelab templates repo, which has no CI; the templates repo only deploys it. Tailscale runs inside the muster container (userspace networking, `--ssh`) rather than as a sidecar, so SSH sessions land where herdr and muster run.
- In the image, `gh` and `muster` are wrappers that load `/run/muster/env` (non-interactive Tailscale SSH commands get neither the pod environment nor `/etc/profile.d`), and git's credential helper points at the `gh` wrapper instead of what `gh auth setup-git` writes (`/usr/bin/gh`).
- Each cloned repo (and the repos and worktrees dirs) is marked trusted in `~/.claude.json` at pod start: the first Claude Code start in an untrusted repo shows a trust prompt, which muster reports as `agent_not_ready` (found on the first real run). Worktrees inherit their main repo's trust.
- The image ships rustup but no toolchain. Toolchains go into rustup's default homes (`~/.rustup`, `~/.cargo` under `/var/lib/muster/home`, on the persistent volume) rather than a custom `RUSTUP_HOME`/`CARGO_HOME`: non-interactive Tailscale SSH commands get neither the pod environment nor `/etc/profile.d`, so a custom home made rustup install a second toolchain there (#8). Every context runs as `muster` with that `HOME`, so the defaults agree. The entrypoint runs `rustup toolchain install` in each repo with a `rust-toolchain` file, so agents can run the checks a definition of done asks for.
- The brief sources (`--brief`, `--brief-file`, `--brief-stdin`) are one required, non-multiple clap `ArgGroup` (`brief_source`); `--brief --detach` goes through the same `<log_dir>/<run>.brief.md` + `--brief-file` handoff as stdin so the child's argv stays short.
