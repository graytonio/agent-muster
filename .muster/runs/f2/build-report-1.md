## Changes

- `src/cli.rs`: added `--brief <TEXT>` to `StartArgs`. Added a required, non-multiple `ArgGroup` `brief_source` over `brief`, `brief_file` and `brief_stdin`. Removed `conflicts_with` and `required_unless_present` from `brief_file`.
- `src/main.rs`: added `brief_source(&StartArgs) -> Brief`. `cmd_start` and `detach` both use it. In detach, an inline brief is written to `<log_dir>/<id>.brief.md` and the child is re-exec'd with `--brief-file`, as for stdin.
- `tests/engine.rs`: added `inline_brief_starts_run`, `inline_brief_detach` and `brief_source_conflicts_exit_2`. The last covers the three conflict pairs (with a "cannot be used with" check) and empty or whitespace `--brief` (exit 2, "the brief is empty").
- `docs/USAGE.md`, `README.md`, `docs/DECISIONS.md`: updated the quick start, SSH examples, `start` row and decision lines.

## Checks

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`: **NOT RUN.** There is no Rust toolchain in this environment (`cargo` and `rustup` are not on PATH and not found anywhere on disk). The code and tests are unverified. They may have compile, formatting (e.g. the multi-line `e.muster(&[...])` call in `inline_brief_starts_run`) or test failures. Please run `just check`.
- The manual clap checks from plan step 1 were also not run.

## Deviations

- Checks could not be run (see above).
- The `--brief-file` PATH canonicalization branch in `detach` is unchanged, so `--brief` takes the `None` arm, which the plan described.
