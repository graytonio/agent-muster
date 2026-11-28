Use rustup's default homes in the container so SSH sessions and agents share one toolchain

Fixes #8 (https://github.com/graytonio/agent-muster/issues/8). Put the line `Fixes #8` at the top of the plan's `## Goal` section so it ends up in the PR body.

## Problem

`container/Dockerfile` sets `ENV RUSTUP_HOME=/var/lib/muster/rustup` and `CARGO_HOME=/var/lib/muster/cargo`. The entrypoint, the herdr server and the agents in its panes inherit them. A non-interactive Tailscale SSH command (`ssh muster@host 'cargo …'`) doesn't, because it gets neither the pod environment nor `/etc/profile.d`. rustup then falls back to `~/.rustup` and `~/.cargo` and auto-installs a second copy of the toolchain there. The muster user's home is `/var/lib/muster/home`, which is on the same persistent volume.

## Requirements

- Drop the custom `RUSTUP_HOME` and `CARGO_HOME` everywhere and use rustup's defaults, `~/.rustup` and `~/.cargo` under `/var/lib/muster/home`. Every context resolves those the same way, because all of them run as the `muster` user with `HOME=/var/lib/muster/home`.
- Places to update:
  - `container/Dockerfile`: `ENV` and the `PATH` entry for cargo-installed binaries, which becomes `/var/lib/muster/home/.cargo/bin`.
  - `container/entrypoint.sh`: the data dirs it creates, and the variables it saves to `/run/muster/env`.
- Keep the rustup binary and its proxies (`cargo`, `rustc`, …) in `/usr/local/bin`, as today. They work with the default homes.
- Leave the existing `/var/lib/muster/rustup` and `/var/lib/muster/cargo` directories on the volume alone, and don't delete them in the entrypoint. Mention in the build report that they can be removed by hand.
- The smoke test in `.github/workflows/release.yml` must check that a session without the pod's environment uses the same rustup home as the entrypoint and the agents. For example, compare `rustup show home` (or `echo $RUSTUP_HOME` / the default) from `docker exec -u muster smoke env -i HOME=/var/lib/muster/home PATH=/usr/local/bin:/usr/bin:/bin rustup show home` against `docker exec -u muster smoke rustup show home`. Pick a check that really fails on the old behaviour.
- Update the matching line in `docs/DECISIONS.md` (the one that says `RUSTUP_HOME`/`CARGO_HOME` are on the persistent volume) and the README if it mentions them.

## Constraints

- You cannot build or run the Docker image in this environment. There is no Docker. Make the edits carefully and reason through them. The PR's CI (the `container image` job in `release.yml`, which runs on PRs that touch `container/**`) builds the image and runs the smoke test.
- The scripts must stay POSIX `sh` or `bash` as they are today, and keep passing `shellcheck` (CI doesn't run shellcheck, but keep them clean). If `shellcheck` is not installed, say so in the build report.

## Definition of done

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` still pass. The Rust code shouldn't change, but run them anyway. The build report lists each changed line in the container files and why.
