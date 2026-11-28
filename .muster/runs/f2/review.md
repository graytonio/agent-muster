VERDICT: APPROVE

## Issues

None.

## Notes

- Checks were re-run by the reviewer with Rust 1.100.0 from `rust-toolchain.toml`:
  - `cargo fmt --check`: pass.
  - `cargo clippy --all-targets -- -D warnings`: pass, no warnings.
  - `cargo test`: pass. 37 unit tests and 7, 2, 19 and 2 integration tests passed, with 0 failures. The 19 in `tests/engine.rs` include `inline_brief_starts_run`, `inline_brief_detach` and `brief_source_conflicts_exit_2`.
  These match `build-report.md`.
- Review 1 issues:
  1. Checks not run: resolved. The toolchain is now installed and all three checks pass (above).
  2. `tests/engine.rs` rustfmt layout: resolved in `d5e47da`. The call is on one line and `cargo fmt --check` passes.
  - The optional note about the `detach` `None` arm is done: `src/main.rs:144` has a comment saying the brief there is inline or stdin.
- The change still matches the brief and plan:
  - `brief_source` is a required, non-multiple `ArgGroup`. Two brief flags, or none, exit 2.
  - An empty or whitespace-only `--brief` is rejected through the shared `Brief::read()`.
  - `--detach` uses the same `<log_dir>/<id>.brief.md` handoff as stdin.
  - The docs and `DECISIONS.md` are updated.
- The human notes for this run ask for `cargo fmt` and a commit. Those steps are for the builder. The reviewer role may not edit code, so this review only confirmed that `cargo fmt --check` already passes and that no formatting commit is needed.
- `git diff main...HEAD` also shows the container, release and version-bump changes from `a30ca27` (#4). That commit is on the remote `main` but missing from the local `main` ref, and it is not part of this change.
