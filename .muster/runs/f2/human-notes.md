The Rust toolchain is now installed on this server. The server image now ships rustup, and Rust 1.100.0 from `rust-toolchain.toml` is installed with clippy and rustfmt, so `cargo` works in this worktree.

Please do the following:
1. Run `cargo fmt`. This fixes the layout the review flagged in `tests/engine.rs`.
2. Run the three checks from the definition of done: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`. Fix anything that fails, and commit.
3. Address the rest of the review (`review.md` issue 2, and the optional comment in the `detach` arm), then record the real check results in `build-report.md`.

`just` is not installed, so run the cargo commands directly.
