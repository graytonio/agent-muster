## Changes

- `container/Dockerfile`
  - Comment above `ARG RUSTUP_VERSION`: toolchains now install into rustup's default home (`~/.rustup`), which is the same in every context; the old wording mentioned `RUSTUP_HOME`.
  - `ENV`: removed `RUSTUP_HOME=/var/lib/muster/rustup` and `CARGO_HOME=/var/lib/muster/cargo`, so every context uses rustup's defaults.
  - `ENV PATH`: trailing `/var/lib/muster/cargo/bin` became `/var/lib/muster/home/.cargo/bin` (cargo-installed binaries). Rustup and its proxies stay in `/usr/local/bin`; the download, checksum and symlinks are untouched.
- `container/entrypoint.sh`
  - Section 1: `install -d` now creates `"$HOME_DIR/.rustup" "$HOME_DIR/.cargo"` (owned by muster) instead of `${RUSTUP_HOME:-$DATA/rustup}` / `${CARGO_HOME:-$DATA/cargo}`. Added a comment saying the old dirs are left in place.
  - Section 2: `RUSTUP_HOME CARGO_HOME` removed from `env_vars`, so they are no longer saved to `/run/muster/env`. `PATH` is still saved, now carrying the new cargo bin dir.
- `.github/workflows/release.yml` (Smoke test step): added checks that
  - `rustup show home` in an `env -i HOME=/var/lib/muster/home PATH=...` session equals `/var/lib/muster/home/.rustup` and equals the `docker exec -u muster` value. On the old image the second value is `/var/lib/muster/rustup`, so this fails.
  - the herdr server's `/proc/<pid>/environ` and `/run/muster/env` contain no `RUSTUP_HOME`/`CARGO_HOME`. They use `if ...; then exit 1; fi`, not a bare `!`, so `bash -e` fails the step.
- `docs/DECISIONS.md`: rewrote the rustup line to record the default-homes decision and why (#8).
- README, `docs/RUNTIME.md` and `docs/USAGE.md` don't mention these variables, so they're unchanged.

## Checks

- `cargo fmt --check`: pass
- `cargo clippy --all-targets -- -D warnings`: pass
- `cargo test`: pass (37 passed)
- `bash -n container/entrypoint.sh` and `sh -n container/env-wrapper.sh`: pass
- grep of `container/` for `RUSTUP_HOME|CARGO_HOME|/var/lib/muster/cargo|/var/lib/muster/rustup|DATA/rustup|DATA/cargo`: only the explanatory comment in `entrypoint.sh`.
- `shellcheck` is not installed here, so it was not run.
- Not run: no Docker here, so the image was not built and the smoke test did not run. No YAML parser is available either (no python3), so the workflow was only checked by eye. The PR's `container image` job builds the image and runs the smoke test.

## Deviations

None.

## Notes

- `/var/lib/muster/rustup` and `/var/lib/muster/cargo` on existing volumes are no longer used and can be removed by hand: `rm -rf /var/lib/muster/rustup /var/lib/muster/cargo`. The entrypoint doesn't touch them. The first start of the new image reinstalls the pinned toolchains into `~/.rustup` (one-time download).
- Deployments that set `RUSTUP_HOME`/`CARGO_HOME` themselves (e.g. homelab Kubernetes templates) would reintroduce the split; worth checking.
