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

## CI

`.github/workflows/ci.yml` runs on every PR (and pushes to `main`): fmt + clippy, tests on Linux and macOS, and a static `x86_64-unknown-linux-musl` build. The `ci` job aggregates them and is the one check to require in branch protection. Run `just check` locally before pushing.

The toolchain is pinned in `rust-toolchain.toml`; rustup installs it automatically. Bump it in its own PR and fix any new lints there.

## Releasing

1. In a PR, bump `version` in `Cargo.toml` (and `Cargo.lock` via `cargo check`), and merge it.
2. Tag the merge commit on `main` and push the tag: `git tag v0.2.0 && git push origin v0.2.0`.
3. `.github/workflows/release.yml` checks that the tag matches `Cargo.toml`, runs the full CI suite, builds `muster` for x86_64/aarch64 Linux (static musl) and x86_64/aarch64 macOS, and publishes a GitHub release with `muster-<version>-<target>.tar.gz` archives (binary plus `RUNTIME.md` and `USAGE.md`) and `SHA256SUMS`. A tag with a `-` (e.g. `v0.2.0-rc.1`) is published as a prerelease.
