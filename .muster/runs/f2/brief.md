Add `--brief <text>` to `muster start` for short inline briefs

## Goal

Today `muster start` takes the brief only from a file (`--brief-file <path>`) or standard input (`--brief-stdin`). For short prompts that is clumsy, especially over SSH. Add a third option that takes the brief text directly as an argument:

    muster start bugfix --repo foo --brief "Fix the flaky retry test in src/net.rs"

## Requirements

- New flag `--brief <TEXT>` on `muster start` (`StartArgs` in `src/cli.rs`).
- Exactly one of `--brief`, `--brief-file` and `--brief-stdin` must be given. Use clap so that passing two of them, or none, is a usage error (exit code 2), with a clear message. Today `--brief-file` and `--brief-stdin` use `conflicts_with` and `required_unless_present`. Extend that cleanly, for example with an `ArgGroup` that is required and not multiple.
- The inline brief must behave exactly like the other sources from then on:
  - it is written to `.muster/runs/<run>/brief.md`
  - its first line becomes the PR title
  - it is used by `--detach` (check how `src/main.rs` hands a stdin brief to the detached re-exec, and make `--brief` work the same way)
  - it is visible in `muster status`.
- Reject an empty or whitespace-only `--brief` the same way an empty brief file or empty stdin is rejected today (if they are). If they aren't, reject all three consistently.
- Keep the change small and focused. No changes to workflows, prompts or the engine beyond what this needs.

## Tests

Add tests alongside the existing ones (`tests/cli_config.rs` and/or `tests/engine.rs`, using `fake-herdr` as the other tests do):
- `--brief "text"` starts a run whose `brief.md` contains the text, and whose PR title is its first line.
- Each conflicting combination (`--brief` with `--brief-file`, `--brief` with `--brief-stdin`) and passing no brief at all exits 2.
- `--brief` with `--detach` works.

## Docs

Update the `start` row and the quick-start examples in `docs/USAGE.md`, the usage section of `README.md`, and add one line to `docs/DECISIONS.md` if you make a non-obvious choice.

## Definition of done

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` all pass.
