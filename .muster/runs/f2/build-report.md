## Changes

No new code changes this iteration. The fixes for the review were already committed in `d5e47da` ("review 1: fix rustfmt layout in test, comment detach arm"):

- `tests/engine.rs`: the `e.muster(&["start", ..., "--brief", text])` call is on one line.
- `src/main.rs`: a comment in the `detach` `None` arm says the brief there is inline or stdin.

`cargo fmt` made no further changes.

## Checks

Run with the toolchain now installed (Rust 1.100.0 from `rust-toolchain.toml`). `just` is not installed, so the cargo commands were run directly.

- `cargo fmt --check`: pass.
- `cargo clippy --all-targets -- -D warnings`: pass, no warnings.
- `cargo test`: pass. Results: 37 unit tests, 7, 2, 19 and 2 integration tests passed, with 0 failures.

## Deviations

None.

## Review responses

1. No checks were run: fixed. The toolchain is installed and all three checks pass (see Checks).
2. `tests/engine.rs` fmt layout: fixed. `cargo fmt --check` passes.
3. Optional comment in the `detach` arm: done, at `src/main.rs:144`.
