# muster task runner. `just --list` shows the recipes.

linux_targets := "x86_64-unknown-linux-musl aarch64-unknown-linux-musl"

default:
    @just --list

# Debug build
build:
    cargo build

# Unit and integration tests
test:
    cargo test

# Formatting and clippy, warnings as errors
lint:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings

# Everything CI should run
check: lint test

# Release binaries: static Linux (musl, via cargo-zigbuild) plus the host macOS build, copied to dist/
release:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p dist
    for t in {{linux_targets}}; do
        cargo zigbuild --release --bin muster --target "$t"
        cp "target/$t/release/muster" "dist/muster-$t"
    done
    host=$(rustc -vV | sed -n 's/^host: //p')
    cargo build --release --bin muster
    cp target/release/muster "dist/muster-$host"
    ls -l dist

# Host-only release build (no cross toolchain needed)
release-host:
    cargo build --release --bin muster

# One-time setup for `just release`: musl targets and cargo-zigbuild (needs zig on PATH)
release-setup:
    rustup target add {{linux_targets}}
    cargo install --locked cargo-zigbuild
    @command -v zig >/dev/null || echo "install zig too (e.g. brew install zig)"
