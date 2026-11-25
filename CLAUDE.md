# muster

Rust CLI that runs multi-agent workflows on a herdr server. See `PLAN.md` for the design and `docs/` for usage, the runtime contract and decisions.

## Build and verify

- Build: `cargo build`
- Verify before every commit (all three must pass):
  - `cargo test`
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo fmt --check`
- `just` recipes wrap these: `just test`, `just lint`, `just build`, `just release`.

## Conventions

- Every herdr CLI call goes through `src/herdr.rs`, behind the `Herdr` trait. No other module shells out to herdr.
- Every git and gh call goes through `src/git.rs`.
- The config dir is read-only. Never write to it. After `start`, read only from the run's config snapshot.
- Never retry an agent prompt automatically. Failures escalate to a human.
- Check herdr flags against the installed binary (`herdr <group>` with no subcommand prints usage; `herdr api schema --json`). If the binary and the docs disagree, the binary wins; note it in `docs/DECISIONS.md`.
- Record each non-obvious choice as one line in `docs/DECISIONS.md`.
- Tests that need herdr use `fake-herdr` (`tests/fake-herdr/`) through `HERDR_BIN`. Tests that need gh use a fake script through `GH_BIN`.
- Never commit secrets, tokens or `.env` files.
