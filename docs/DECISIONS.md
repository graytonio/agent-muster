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
