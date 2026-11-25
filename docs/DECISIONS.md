# Decisions

One line per non-obvious choice, with the reason.

## Project

- Crate lives at the repo root (not `muster/`), binary name `muster`. The repo is the project; a nested dir adds nothing.
- `fake-herdr` is a second binary target (`tests/fake-herdr/main.rs`) so integration tests get `CARGO_BIN_EXE_fake-herdr`. Release recipes build `--bin muster` only.
- `rust-version = 1.89` because run locks use `std::fs::File::try_lock` (stable in 1.89), which avoids a locking crate.
- Added `sha2` for the config-snapshot content hash.

## Operator answers to the plan's open questions

- Reviewer is Claude (opus) by default, not Codex. Removes `codex` from the runtime contract's required list (still a valid `kind`).
- No `tutorial` stage in the default workflows. Optional stages and `--skip` are still supported.
- PRs open ready for review (not drafts). `[finish] draft = true` is available per workflow.
