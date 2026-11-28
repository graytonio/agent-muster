# Plan: use rustup's default homes in the container (f4)

## Goal

Fixes #8

Every context in the server image (the entrypoint, the herdr server, agents in its panes, login SSH shells and non-interactive `ssh muster@host 'cargo …'` commands) must resolve the same rustup and cargo homes, so a toolchain is installed once and shared.

## Summary

The image sets `RUSTUP_HOME=/var/lib/muster/rustup` and `CARGO_HOME=/var/lib/muster/cargo` with `ENV`, which non-interactive Tailscale SSH commands don't get, so rustup there falls back to `~/.rustup`/`~/.cargo` and installs a second toolchain. We drop both variables from `container/Dockerfile` and `container/entrypoint.sh` and rely on rustup's defaults under the muster user's home (`/var/lib/muster/home`, on the persistent volume), which every context resolves the same way because all of them run as `muster` with that `HOME`. The release smoke test gets a check that fails on the old behaviour, and `docs/DECISIONS.md` is updated. No Rust code changes.

## Steps

1. **`container/Dockerfile`: drop the custom homes.**
   - In the final `ENV` block (lines 90–95), delete `RUSTUP_HOME=/var/lib/muster/rustup \` and `CARGO_HOME=/var/lib/muster/cargo \`.
   - In the same block, change the trailing `PATH` entry `/var/lib/muster/cargo/bin` to `/var/lib/muster/home/.cargo/bin` (for `cargo install`ed binaries). Keep the rest of `PATH` unchanged, so `/usr/local/bin` (rustup and its proxies) still comes first.
   - Update the comment above `ARG RUSTUP_VERSION` (lines 22–24): replace "onto the persistent volume (RUSTUP_HOME)" with wording like "into rustup's default home, `~/.rustup` under the muster user's home on the persistent volume". Mention why: it is the same in every context, including SSH commands that get no pod environment.
   - Do not touch the rustup download, its checksum, or the proxy symlinks in `/usr/local/bin`.

2. **`container/entrypoint.sh`: data dirs and SSH environment.**
   - Section 1 (`install -d -o muster -g muster ...`, lines 35–37): replace `"${RUSTUP_HOME:-$DATA/rustup}" "${CARGO_HOME:-$DATA/cargo}"` with `"$HOME_DIR/.rustup" "$HOME_DIR/.cargo"`. These dirs belong to `muster`, so rustup can write there when it runs as muster. Do not create, touch or delete `$DATA/rustup` or `$DATA/cargo`.
   - Section 2 (`env_vars=(...)`, lines 45–47): remove `RUSTUP_HOME CARGO_HOME` from the list and keep `PATH`. `PATH` now carries `/var/lib/muster/home/.cargo/bin` from the image `ENV` into `/run/muster/env`.
   - Optionally add a short comment by the install line: "rustup's default homes (~/.rustup, ~/.cargo): every context runs as muster with this HOME, so they agree even in SSH commands with no pod environment. Older images used $DATA/rustup and $DATA/cargo; they're left in place."
   - Keep the script valid bash and shellcheck-clean (no new unquoted expansions). If `shellcheck` is available run `shellcheck container/*.sh`; it is **not** installed in this environment (checked), so say so in the build report.

3. **`.github/workflows/release.yml`: smoke test for a shared rustup home.**
   In the `Smoke test` step, after the existing `rustup --version` line, add a check like:
   ```sh
   # A session with no pod environment (like a non-interactive SSH command)
   # must use the same rustup home as docker exec, the herdr server and its agents.
   bare_home=$(docker exec -u muster smoke env -i HOME=/var/lib/muster/home PATH=/usr/local/bin:/usr/bin:/bin rustup show home)
   pod_home=$(docker exec -u muster smoke rustup show home)
   echo "bare: $bare_home, pod: $pod_home"
   test "$bare_home" = /var/lib/muster/home/.rustup
   test "$pod_home" = "$bare_home"
   # Neither the herdr server (whose environment the agents inherit) nor the
   # saved SSH environment overrides the homes.
   herdr_pid=$(docker exec smoke pgrep -o -u muster -x herdr)
   if docker exec smoke sh -c "tr '\0' '\n' < /proc/$herdr_pid/environ" | grep -E '^(RUSTUP|CARGO)_HOME='; then exit 1; fi
   if docker exec smoke grep -E '(RUSTUP|CARGO)_HOME' /run/muster/env; then exit 1; fi
   ```
   Why it fails on the old image: `docker exec -u muster smoke rustup show home` inherits the image `ENV RUSTUP_HOME` and prints `/var/lib/muster/rustup`, while the `env -i` session prints `/var/lib/muster/home/.rustup`; the herdr environ and `/run/muster/env` also contained `RUSTUP_HOME`. The builder may adjust the exact form (e.g. a single `docker exec smoke sh -c '...'` per check), but must keep: (a) the `env -i` vs. pod-environment comparison, (b) the literal expected path, and (c) the herdr server environment check. `run:` steps use `bash -e`, so a `!` pipeline that finds a match fails the step only if written so it is not exempt from `-e`: use `if ...; then exit 1; fi` (or `test -z "$(...)"`) rather than a bare `! cmd`, because bash ignores `-e` for `!` pipelines. Note `pgrep` comes from `procps`, which the image installs; `docker exec smoke` (no `-u`) runs as root so it can read `/proc/<pid>/environ`.

4. **`docs/DECISIONS.md`: update the rustup line (line 87).**
   Replace "`RUSTUP_HOME`/`CARGO_HOME` are on the persistent volume" with something like: "Toolchains go into rustup's default homes (`~/.rustup`, `~/.cargo` under `/var/lib/muster/home`, on the persistent volume) rather than a custom `RUSTUP_HOME`/`CARGO_HOME`: non-interactive Tailscale SSH commands get neither the pod environment nor `/etc/profile.d`, so a custom home made rustup install a second toolchain there (#8). Every context runs as `muster` with that `HOME`, so the defaults agree." Keep the rest of the line (entrypoint runs `rustup toolchain install` per repo).
   README: checked, it does not mention `RUSTUP_HOME`/`CARGO_HOME` (only "rustup with a C toolchain"); no change needed. `docs/RUNTIME.md` and `docs/USAGE.md` don't mention them either.

5. **Verify and write the build report.**
   Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` (or `just check`). In the build report: list each changed line in `container/Dockerfile`, `container/entrypoint.sh` and `release.yml` with the reason; state that the image was not built locally (no Docker) and the PR's `container image` job runs the smoke test; state that `shellcheck` is not installed; and say that `/var/lib/muster/rustup` and `/var/lib/muster/cargo` on existing volumes are no longer used and can be removed by hand (`rm -rf /var/lib/muster/rustup /var/lib/muster/cargo`) — the first start of the new image reinstalls the pinned toolchains into `~/.rustup` (expected one-time download).

Steps 1–4 can be a single commit if the builder prefers (they are one logical change and the smoke test only makes sense with it), or one commit each.

## Definition of done

- [ ] `grep -rn 'RUSTUP_HOME\|CARGO_HOME\|/var/lib/muster/cargo\|/var/lib/muster/rustup\|DATA/rustup\|DATA/cargo' container/` finds nothing except, optionally, a comment in `entrypoint.sh`/`Dockerfile` explaining that the old dirs are left in place.
- [ ] `container/Dockerfile` `ENV` has no `RUSTUP_HOME`/`CARGO_HOME`, and `PATH` ends with `/var/lib/muster/home/.cargo/bin`; rustup and its proxies are still installed in `/usr/local/bin`.
- [ ] `container/entrypoint.sh` creates `$HOME_DIR/.rustup` and `$HOME_DIR/.cargo` owned by `muster`, does not create or delete `$DATA/rustup`/`$DATA/cargo`, and no longer saves `RUSTUP_HOME`/`CARGO_HOME` to `/run/muster/env` (still saves `PATH`).
- [ ] `bash -n container/entrypoint.sh` and `sh -n container/env-wrapper.sh` pass; `shellcheck container/*.sh` passes if available (otherwise noted in the build report).
- [ ] The `Smoke test` step in `.github/workflows/release.yml` compares `rustup show home` from an `env -i HOME=/var/lib/muster/home PATH=/usr/local/bin:/usr/bin:/bin` session against `docker exec -u muster smoke rustup show home`, asserts it equals `/var/lib/muster/home/.rustup`, and asserts the herdr server's environment has no `RUSTUP_HOME`/`CARGO_HOME`; each assertion actually fails the step under `bash -e` (no bare `! cmd`).
- [ ] The workflow YAML still parses (e.g. `python3 -c 'import yaml,sys; yaml.safe_load(open(".github/workflows/release.yml"))'` if PyYAML is available).
- [ ] `docs/DECISIONS.md` line about `RUSTUP_HOME`/`CARGO_HOME` updated; README unchanged (it doesn't mention them).
- [ ] No Rust source changes; `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass.
- [ ] Build report lists each changed container/workflow line and why, notes no Docker/no shellcheck, and says the old `/var/lib/muster/rustup` and `/var/lib/muster/cargo` dirs can be removed by hand.

## Risks

- **`## Goal` vs. required sections.** The brief asks for `Fixes #8` at the top of a `## Goal` section, while the planner template lists Summary/Steps/DoD/Risks. I added `## Goal` (with `Fixes #8` first) ahead of the required sections.
- **HOME under `docker exec -u muster`.** The smoke check assumes Docker sets `HOME` from `/etc/passwd` (`/var/lib/muster/home`) for `docker exec -u muster` when the image sets no `HOME`, which is Docker's behaviour. If it didn't, `pod_home` would differ and the check would fail loudly rather than pass silently — acceptable.
- **`MUSTER_DATA` other than `/var/lib/muster`.** The entrypoint derives `HOME_DIR=$DATA/home`, but the muster user's passwd home is fixed at `/var/lib/muster/home`. SSH sessions use the passwd home, so with a non-default `MUSTER_DATA` the entrypoint and SSH could still disagree. That's pre-existing (the volume is always `/var/lib/muster` in practice) and out of scope; the cargo `PATH` entry is hardcoded to `/var/lib/muster/home/.cargo/bin` as the brief says.
- **Deployments that set `RUSTUP_HOME`/`CARGO_HOME` themselves** (e.g. the homelab Kubernetes templates) would reintroduce the split. Assumed they don't; out of scope for this repo. Note it in the build report as something to check.
- **First start after upgrade** reinstalls pinned toolchains into `~/.rustup` (the old ones in `/var/lib/muster/rustup` are ignored). Expected; the entrypoint already handles install failures as warnings.
- **`rustup` as a proxy without `~/.cargo/bin/rustup`.** rustup run from `/usr/local/bin` with default homes works the same as with custom ones (only `rustup self` commands care about the install location), so no change there. Assumed, can't be checked without Docker; the smoke test's `rustup show home` and the entrypoint's `rustup toolchain install` cover it in CI.
- **Herdr pid lookup** in the smoke test assumes a single long-lived `herdr` process owned by `muster` (`pgrep -o` picks the oldest, the server). If herdr forks helpers with the same name, the oldest is still the server.
