VERDICT: APPROVE

## Issues

None.

## Notes

Reviewed against the branch base `1ca440b`. The local `main` ref is stale (it sits at #3), so `git diff main...HEAD` also shows #4–#6; those aren't part of this change.

Checked:
- `container/Dockerfile`: `RUSTUP_HOME` and `CARGO_HOME` are gone from `ENV`, and the `PATH` tail is now `/var/lib/muster/home/.cargo/bin`. The rustup download, its checksum and the `/usr/local/bin` proxies are untouched. The comment is updated.
- `container/entrypoint.sh`: it now creates `$HOME_DIR/.rustup` and `$HOME_DIR/.cargo`, owned by muster. The old `$DATA/rustup` and `$DATA/cargo` are left alone. `RUSTUP_HOME`/`CARGO_HOME` are dropped from `env_vars`. Every muster context gets `HOME=/var/lib/muster/home`: `as_muster` sets it, `docker exec -u muster` takes it from passwd, and the herdr panes inherit it. So the defaults resolve the same way everywhere.
- The smoke test really fails on the old image. There, `docker exec -u muster smoke rustup show home` returns `/var/lib/muster/rustup`, while the bare `env -i` session returns `/var/lib/muster/home/.rustup`, so the `test "$pod_home" = "$bare_home"` line fails. The negative greps use `if …; then exit 1; fi`, which works under Actions' `bash -e`. An empty `pgrep` result also fails the step, because a failed command substitution in an assignment trips `-e`.
- `docs/DECISIONS.md`: the line is updated. README, RUNTIME.md and USAGE.md don't mention the variables. The plan's Goal starts with `Fixes #8`.
- I ran the checks myself: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` all pass. No Rust code changed. `shellcheck` isn't installed here, which matches the report.
- The build report lists each changed line and gives the manual-removal command for the old dirs.

Non-blocking suggestions:
- The herdr `/proc/<pid>/environ` check would also pass if the `docker exec … tr` call failed, for example on an unreadable environ: `grep` would see no input and the step would go on. Reading the environ into a variable first, so a read failure fails the step, would make it stricter. That said, `herdr_pid` is already validated.
- As the report says, any deployment template (e.g. the homelab Kubernetes manifests) that sets `RUSTUP_HOME`/`CARGO_HOME` itself would bring the split back. Worth checking when this image is rolled out.
