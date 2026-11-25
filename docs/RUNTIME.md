# Runtime contract

What a deployment must provide for `muster` to run. The homelab templates repo implements this; `muster doctor` checks every item and exits non-zero if any fails, so it can be used as a startup or readiness check.

`muster` runs on the same host (in practice, the same pod) as the herdr server and drives it through the `herdr` CLI. Nothing is installed on client machines; operators reach it over Tailscale SSH.

## Binaries

| Requirement | Notes | doctor check |
|---|---|---|
| `herdr` on `PATH`, or `HERDR_BIN` | Checked against herdr 0.9.3. | `herdr binary` |
| A running herdr server for the target session | `[herdr] session` in `muster.toml`; empty means the default session. | `herdr server` |
| herdr integrations for every agent kind the workflows use | `herdr integration install claude` (and `codex` if a workflow uses it). | `herdr integration <kind>` |
| Agent CLIs on `PATH`, already authenticated | `claude` for the default workflows. `codex` only if a workflow uses `kind = "codex"`. | `agent cli <bin>` |
| `git` on `PATH`, able to push to muster repos | Credentials for the remote (e.g. a credential helper using `GH_TOKEN`). | `git` |
| `gh` on `PATH` (or `GH_BIN`), authenticated | For example `GH_TOKEN` in the environment. | `gh`, `gh auth` |
| Optional: `entire` on `PATH` (or `ENTIRE_BIN`) | Only when `[entire] enabled = true`. | `entire` |

The agent CLIs' first-run trust and onboarding prompts must already be dealt with for the user muster runs as, or `agent start` returns `agent_not_ready` and the run escalates. (What exactly is needed is still to be recorded; see "Manual run" in `USAGE.md`.)

## Config directory (read-only)

- Default `/etc/muster`. Overridden by `MUSTER_CONFIG_DIR`, then by `--config-dir`.
- Contains `muster.toml`, `workflows/*.toml` and `prompts/*.md`. `muster init --out <dir>` writes a starting set.
- Expected to be a mounted ConfigMap. **Mount the whole directory, not individual files with `subPath`**: `subPath` mounts don't receive updates.
- `muster` never writes to it. Files may be symlinks into a hidden `..data` directory; muster follows symlinks and ignores entries starting with `.`.
- It can change at any time (ArgoCD sync). Each run snapshots the workflow and prompts it uses at `start`, so a config change never affects a run in flight.
- CI for the templates repo can run `muster validate --config-dir <path>`, which needs no herdr, git or network.

doctor checks: `config dir`, `muster.toml`, `workflow <name>`.

## Writable data directories (persistent volume)

Set in `muster.toml`; `~` is expanded.

| Key | Holds | doctor check |
|---|---|---|
| `state_dir` | Run state (`<run>.json`), locks, config snapshots, assembled PR bodies | `state_dir` |
| `log_dir` | Logs of `--detach` runs | `log_dir` |
| `repos_dir` | Target repos, already cloned as `<repos_dir>/<repo>` | `repos_dir` |
| `worktrees_dir` (optional) | Run worktrees. Empty means herdr's default, `~/.herdr/worktrees`, which must then be on the persistent volume too. | `worktrees_dir` |

Every repo named under `[repos.<name>]` must be cloned (`repo <name>`). Repos without overrides need no entry; `start --repo <name>` only requires `<repos_dir>/<name>/.git` to exist.

## Process

- `muster` and `herdr` run as the **same user**, so `muster` can reach herdr's socket.
- Concurrent runs are limited by `max_concurrent_runs`. Each run holds a lock file in `state_dir` while a process drives it.
- `muster start --detach` re-executes the same binary in the background; the binary path must stay valid while runs are in flight.
