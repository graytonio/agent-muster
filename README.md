# muster

muster runs multi-agent coding workflows on a [herdr](https://herdr.dev) server. A workflow is a sequence of stages, such as plan → build → review. Each stage is an AI agent (Claude Code or Codex) in its own herdr pane. Stages hand work to each other through files, and a run ends with a pull request. Every run leaves a paper trail on its branch: the brief, every rendered prompt, and every stage's output.

```sh
muster start feature --repo my-service --brief-file brief.md
muster status f1
```

Workflows and role prompts are plain TOML and Markdown in a read-only config directory. Adding a new kind of task means adding files, not changing code. Runs are resumable. muster never retries an agent on its own: when a stage fails, the run stops and waits for a human.

## Install

muster runs on the same machine as the herdr server it drives. Usually that's a server or a pod, reached over SSH. See [docs/RUNTIME.md](docs/RUNTIME.md) for everything the host needs.

### Prebuilt binaries

Each [release](https://github.com/graytonio/agent-muster/releases) has these builds:

| Platform | Archive |
|---|---|
| Linux x86_64 (static) | `muster-<version>-x86_64-unknown-linux-musl.tar.gz` |
| Linux arm64 (static) | `muster-<version>-aarch64-unknown-linux-musl.tar.gz` |
| macOS Apple silicon | `muster-<version>-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `muster-<version>-x86_64-apple-darwin.tar.gz` |

Download, verify and install, for example on Linux x86_64:

```sh
version=0.1.2
target=x86_64-unknown-linux-musl
base=https://github.com/graytonio/agent-muster/releases/download/v${version}

curl -fsSLO "${base}/muster-${version}-${target}.tar.gz"
curl -fsSLO "${base}/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS   # macOS: shasum -a 256 --check --ignore-missing SHA256SUMS

tar -xzf "muster-${version}-${target}.tar.gz"
sudo install -m 755 "muster-${version}-${target}/muster" /usr/local/bin/muster
muster --version
```

With the GitHub CLI: `gh release download v0.1.2 -R graytonio/agent-muster -p '*x86_64-unknown-linux-musl*' -p SHA256SUMS`.

Each archive also contains `RUNTIME.md` and `USAGE.md`.

### Container image

`ghcr.io/graytonio/muster:<version>` (linux/amd64 and linux/arm64) is a ready-made server. It contains muster, herdr, Claude Code, git, gh and Tailscale. Its entrypoint does the following:

1. Prepares the data directories on a volume mounted at `/var/lib/muster`.
2. Clones the repos listed in `MUSTER_REPOS`.
3. Joins your tailnet with Tailscale SSH enabled.
4. Runs the herdr server as the `muster` user.

To set it up, mount a config directory at `/etc/muster` and set these environment variables:

- `GH_TOKEN`
- `CLAUDE_CODE_OAUTH_TOKEN`
- `TS_AUTHKEY`

[`container/entrypoint.sh`](container/entrypoint.sh) lists every variable it reads. A Kubernetes deployment of the image lives in [homelab-flagops-templates](https://github.com/graytonio/homelab-flagops-templates/tree/main/apps/muster).

### From source

You need Rust 1.89 or newer:

```sh
cargo install --locked --git https://github.com/graytonio/agent-muster --tag v0.1.2 --bin muster muster
```

## Set up a host

1. Install herdr and start its server: `herdr server`. Install the herdr integration for each agent kind: `herdr integration install claude`.
2. Install `claude` (and `codex` if a workflow uses it), `git` and `gh`, and authenticate them.
3. Write a config directory and point muster at it:

   ```sh
   muster init --out ~/muster-config        # muster.toml, workflows/, prompts/
   export MUSTER_CONFIG_DIR=~/muster-config # default: /etc/muster
   ```

   Edit `muster.toml` to set the data directories: `repos_dir`, `state_dir`, `log_dir`, `worktrees_dir`. Then clone the repos you want muster to work on into `repos_dir`.
4. Check the host with `muster doctor`, which checks every item of the runtime contract and prints a fix hint for each failure.

## Use

```sh
muster list-workflows
muster start feature --repo my-service --brief-file brief.md
echo "Fix the flaky retry test" | muster start bugfix --repo my-service --brief-stdin --detach
muster start bugfix --repo my-service --brief "Fix the flaky retry test in src/net.rs"
muster status            # all runs
muster status f1         # one run and its event log
muster resume f1         # after fixing whatever the run escalated
muster clean f1 --delete-branch
```

From another machine, over SSH:

```sh
ssh muster@muster 'muster start feature --repo my-service --brief-stdin --detach' < brief.md
ssh muster@muster 'muster start bugfix --repo my-service --brief "Fix the flaky retry test" --detach'
```

These workflows ship with muster:

| Workflow | Stages |
|---|---|
| `feature` | plan → build → review |
| `bugfix` | reproduce → fix → review |
| `investigate` | a read-only report, with no PR |

Every command takes `--json`. Exit codes:

| Code | Meaning |
|---|---|
| `0` | done |
| `2` | usage or config error |
| `3` | needs a human |
| `4` | failed |
| `5` | concurrency limit reached |

## Docs

- [docs/USAGE.md](docs/USAGE.md): commands, configuration, writing a workflow, and handling escalations
- [docs/RUNTIME.md](docs/RUNTIME.md): what the host must provide
- [docs/DECISIONS.md](docs/DECISIONS.md): why things work the way they do
- [PLAN.md](PLAN.md): the original design

## Development

```sh
just check    # fmt, clippy, tests
just build
```

See [CLAUDE.md](CLAUDE.md) for the conventions, CI, and how releases are made.
