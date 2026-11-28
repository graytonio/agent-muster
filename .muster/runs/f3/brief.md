Add `--detach` to `muster resume`

Fixes #7 (https://github.com/graytonio/agent-muster/issues/7). Put the line `Fixes #7` at the top of the plan's `## Goal` section so it ends up in the PR body.

## Problem

`muster start` takes `--detach`: it re-executes itself in the background, logs to `<log_dir>`, and returns right away. `muster resume` has no such flag, so resuming a run over SSH holds the session open until the run finishes or escalates, which can take tens of minutes. Today `muster resume f2 --detach` fails with `error: unexpected argument '--detach' found`.

## Requirements

- `muster resume <run> --detach` behaves like `start --detach`:
  - it prints the run id (and honours `--json` the way start does), then returns immediately
  - a background process drives the run, logging to the same per-run log file in `log_dir`
- Look at how `start --detach` is implemented in `src/main.rs` and reuse it rather than duplicating it.
- Run-existence checks, the run lock and the concurrency limit behave exactly as for a foreground resume, with the same exit codes. Errors that can be detected up front (an unknown run, a run that isn't resumable, a run already locked by another process) are reported synchronously, before detaching, with the usual exit code.
- No changes to the engine's stage logic.

## Tests

Use `fake-herdr`, the same way the existing `--detach` tests for `start` do:
- `resume --detach` returns promptly and the run then completes in the background.
- `resume --detach` on an unknown run exits with the same code as a foreground resume, without detaching.

## Docs

Update the `resume` row in `docs/USAGE.md`, plus the escalation section where it says to run `muster resume`. If you make a non-obvious choice, add one line to `docs/DECISIONS.md`.

## Definition of done

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` all pass. The Rust toolchain is installed: run the cargo commands directly, since `just` isn't installed.
