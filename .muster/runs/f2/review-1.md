VERDICT: CHANGES

## Issues

1. **The definition of done is not met: no checks were run.** `build-report.md` says `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` were **not run** because there is no Rust toolchain in the environment. The reviewer environment has none either (`cargo`, `rustup` and `rustfmt` are not on PATH or anywhere on disk), so nothing in this change has been compiled or tested. Run all three (`just check`) in an environment that has the toolchain from `rust-toolchain.toml`, fix whatever fails, and record the real results in the build report. If no toolchain can be had, escalate to a human instead of reporting the work as done.

2. **`tests/engine.rs:677-679` will fail `cargo fmt --check`.** The `e.muster(&[ "start", "feature", "--repo", "sandbox", "--brief", text, ])` call is split over three lines, but on one line it is about 82 columns, under rustfmt's 100-column limit, so rustfmt will put it back on one line:
   ```rust
   let o = e.muster(&["start", "feature", "--repo", "sandbox", "--brief", text]);
   ```
   The build report already flagged this as a risk. Run `cargo fmt` to fix it.

## Notes

- Apart from the unverified build, the change looks correct and matches the brief and plan:
  - `brief_source` is a required, non-multiple `ArgGroup` over the three flags. The old `conflicts_with` and `required_unless_present` are removed.
  - `brief_source()` in `src/main.rs` maps the flags to the existing `Brief::Text`, `Brief::File` and `Brief::Stdin`, so the inline brief goes through the same `read()` check for empty or whitespace-only text (exit 2, "the brief is empty").
  - `--detach` reuses the `<log_dir>/<id>.brief.md` and `--brief-file` handoff.
  - The tests cover the brief: `brief.md` contents, the PR title from the first line only, the three conflict pairs, empty and whitespace-only `--brief`, and `--brief --detach`. The existing `usage_errors` test still covers the case where no brief is given.
- `inline_brief_starts_run` checks only `status`/`run_dir` from `muster status`. That matches how the other brief sources appear there, since status shows the run dir rather than the brief text. This is acceptable.
- The `git diff main...HEAD` also shows container, release and version-bump changes. Those come from `a30ca27` (#4), which is on `main` but missing from the local `main` ref. They are not part of this change.
- Possible tidy-up: in `detach`, the `None` arm calls `brief_source(&args)`, which can only return `Text` or `Stdin` there. That is correct, but a comment saying so would help readers.
