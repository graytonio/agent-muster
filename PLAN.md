# muster: build plan

`muster` is a Rust CLI that orchestrates multi-agent workflows on top of a running herdr server. It reads workflow configs and role prompt templates, starts agents in herdr panes, moves work between stages through files, and finishes runs by opening a PR. Every run leaves a paper trail.

Inspired by [Welcome to Glass Town](https://aaif.io/blog/welcome-to-glass-town-building-a-software-factory-with-a-paper-trail).

**Scope of this project:** the `muster` binary, its config format, and the default workflows and prompts that ship with it. Deployment (container image, Kubernetes, Tailscale, secrets) lives in a separate homelab templates repo. This project only defines the runtime contract that deployment must satisfy (see "Runtime contract").

**Where it runs:** `muster` runs on the homelab, in the same pod as the herdr server. It drives herdr through the `herdr` CLI, which talks to the server's local socket. Nothing is installed on client machines. The operator reaches it from any machine over Tailscale SSH: either `herdr --remote muster` and then running `muster ...` in a pane, or one-shot commands like `ssh muster 'muster start feature --repo foo --brief-stdin' < brief.md`.

**Where config lives:** a read-only config directory on the server, populated by Helm/ArgoCD from the templates repo (in practice, a mounted ConfigMap). `muster` never writes to it and has no git or sync logic of its own. Mutable data (run state, logs, worktrees) lives in separate writable directories on the persistent volume.

---

## How to use this plan (instructions for Claude Code)

- Work **one phase at a time**, in order. Each phase ends with acceptance criteria. Don't start the next phase until they pass, or until you've told the human exactly which ones you couldn't verify and why.
- Steps marked **HUMAN** need the operator, for example running agents interactively or creating GitHub repos. When you reach one, stop. Print the exact commands, then wait.
- **Don't invent herdr flags.** The CLI details below come from herdr docs v0.9.5. Before writing code against them, check against the installed binary with `herdr --help`, `herdr <cmd> --help`, and `herdr api schema --json`. If they disagree, the binary wins. Note the difference in `docs/DECISIONS.md`.
- Items marked **VERIFY** are assumptions about third-party tools. Check them against current docs before relying on them.
- Keep a running `docs/DECISIONS.md`: one line per non-obvious choice, with the reason.
- Stack: Rust (edition 2021), `clap` (derive), `serde`, `toml`, `serde_json`, `anyhow`/`thiserror`, `time`. No async runtime unless clearly needed; the conductor is sequential and shells out. No Python.

## Goals

1. `muster start <workflow> --repo <repo> --brief-file <path>` runs a workflow end to end against a herdr server: each stage is an agent in its own pane, stages hand off through required artifact files, and the run finishes with a PR.
2. Workflows and role prompts are data (TOML and Markdown), loaded from a read-only config directory. Adding a new kind of task means adding files to the Helm chart, not changing code. Each run snapshots the config it started with, so a config update mid-run never changes an in-flight run.
3. Runs are inspectable and resumable: persistent run state, `status`, `resume`, and `clean` commands, and a committed run directory on the branch.
4. Cost controls are built in: per-stage timeouts, capped review loops, no automatic retries, and a global concurrency limit.
5. Output is scriptable: every command supports `--json`, so a herdr plugin, a Claude Code skill, or a future trigger service can wrap it later.

## Non-goals

- Deployment: image, Kubernetes manifests, networking, and secrets belong to the homelab templates repo.
- The herdr plugin and the laptop Claude Code skill. These are thin wrappers to build later on top of `muster --json`. The CLI must not need them.
- Automatic triggers (webhooks, cron, alerts).
- Talking to herdr's raw socket protocol. Use the documented CLI.
- Pulling or syncing config from git. Deployment populates the config directory; `muster` just reads it.
- Running on client machines or driving a remote herdr. `muster` and herdr are always on the same host.

---

## Architecture summary

```
muster start ──► load config + workflow + prompts
                  │
                  ├─ herdr worktree create ──► workspace + git worktree on branch muster/<prefix>/<run>
                  │
                  ├─ for each stage:
                  │    pane split → agent start → render prompt to file → agent prompt --wait
                  │    → classify outcome → (next stage | loop back | escalate)
                  │
                  └─ finish: commit leftovers → git push → gh pr create → notify

State:   <state_dir>/<run>.json             (not committed)
Trail:   <worktree>/.muster/runs/<run>/    (committed: brief, artifacts, rendered prompts)
```

Key decisions (already made, don't revisit without asking):

| Decision | Choice | Why |
|---|---|---|
| Orchestration | Deterministic CLI, not an LLM orchestrator | Cheap, predictable, enforces cost caps |
| herdr integration | Shell out to the `herdr` CLI and parse JSON | Documented, stable flags; the socket protocol version can change between releases |
| Handoff between stages | Required artifact file per stage | Conductor can verify completion and the trail is self-documenting |
| Prompts | Rendered to a file; agent gets a short "read this file" prompt | Avoids paste and input-size issues, keeps exact prompts in the trail |
| Failure handling | Escalate to a human, never auto-retry | Failed runs still cost tokens; a stalled agent usually needs a decision |
| Permissions | Agents run with permission prompts skipped (configured per stage in args) | The deployment sandbox is the boundary; otherwise agents sit blocked |

---

## Runtime contract (what deployment must provide)

Document this in `docs/RUNTIME.md` so the homelab templates repo can implement it. `muster doctor` (Phase 7) checks every item.

- `herdr` on `PATH` (or `HERDR_BIN`), with a server running for the target session. Integrations installed: `herdr integration install claude` and `herdr integration install codex`.
- Agent CLIs available to herdr panes (`claude`, `codex`) and already authenticated.
- `git` and `gh` on `PATH`, with `gh` authenticated (for example `GH_TOKEN`) and git able to push to muster repos.
- Optional: `entire` on `PATH` for session checkpoints.
- A **read-only config directory** (default `/etc/muster`, override with `MUSTER_CONFIG_DIR` or `--config-dir`) containing `muster.toml`, `workflows/` and `prompts/`. Expected to be a mounted ConfigMap. Mount the whole directory, not individual files with `subPath`, because `subPath` mounts don't receive updates.
- **Writable data directories** on the persistent volume for `state_dir`, `log_dir` and `repos_dir` (paths set in `muster.toml`).
- Target repos already cloned under `repos_dir`.
- `muster` and `herdr` run as the same user, so `muster` can reach herdr's socket.

---

## Repo layout

```
muster/
├── CLAUDE.md                 # conventions for agents working on THIS repo
├── PLAN.md                   # this file
├── Cargo.toml                # binary name: muster
├── src/
│   ├── main.rs               # clap CLI
│   ├── config.rs             # muster.toml
│   ├── workflow.rs           # workflow TOML schema + validation
│   ├── render.rs             # prompt template rendering
│   ├── herdr.rs              # ALL herdr CLI calls live here, behind a trait
│   ├── git.rs                # git + gh calls
│   ├── state.rs              # run state persistence + locking
│   ├── engine.rs             # stage loop, outcome classification
│   └── doctor.rs
├── defaults/
│   ├── muster.toml          # example config
│   ├── workflows/{feature.toml, bugfix.toml, investigate.toml}
│   ├── prompts/{planner,builder,reviewer,tutorial,reproducer,fixer,investigator}.md
│   └── AGENTS.muster.md     # snippet for target repos' AGENTS.md / CLAUDE.md
├── tests/
│   ├── fake-herdr/           # scripted stand-in for the herdr CLI (Phase 3)
│   ├── fixtures/             # recorded herdr JSON responses (Phase 4)
│   └── *.rs
└── docs/{DECISIONS.md, RUNTIME.md, USAGE.md}
```

---

## Config formats

### Config directory layout

```
<config_dir>/            # read-only; default /etc/muster
├── muster.toml
├── workflows/*.toml
└── prompts/*.md
```

Config directory lookup: `--config-dir <path>`, then `$MUSTER_CONFIG_DIR`, then `/etc/muster`. Workflows and prompts are always resolved relative to the config dir, so the layout is fixed and a Helm chart only has to produce these files.

**ConfigMap mount quirks** (must handle; add tests): files in a mounted ConfigMap are symlinks into a hidden `..data` directory, and Kubernetes swaps that symlink atomically on update. Directory listing must follow symlinks and skip entries starting with `.`, such as `..data` and `..2026_11_25_...`. Read config once per command invocation; never cache it across commands.

### `muster.toml` (global config)

Every path supports `~` expansion. The data directories must be writable and on persistent storage.

```toml
repos_dir = "~/repos"
state_dir = "~/.muster/state"
log_dir   = "~/.muster/logs"
max_concurrent_runs = 2

[herdr]
bin = "herdr"          # overridden by HERDR_BIN
session = ""           # empty = default session; else passed as --session

[git]
remote = "origin"
base_branch = "main"   # per-repo override below

[entire]
enabled = true
checkpoint_remote = "github:<owner>/muster-checkpoints"   # optional

[repos.muster-sandbox]
base_branch = "main"
```

`muster init --out <dir>` writes the default config tree (`muster.toml`, `workflows/`, `prompts/`) to a directory. It's for seeding the Helm chart in the templates repo and for local testing. It refuses to write into an existing non-empty directory unless `--force` is given. It never targets the live config dir.

### Workflow (`workflows/<name>.toml`)

```toml
[workflow]
name = "feature"
description = "Plan, build, review a feature from a brief"
branch_prefix = "muster/feat"

[[stage]]
id = "plan"
role = "planner"               # prompts/planner.md
kind = "claude"                # herdr agent kind
args = ["--model", "opus", "--dangerously-skip-permissions"]
produces = "plan.md"           # relative to run dir
timeout_min = 10

[[stage]]
id = "build"
role = "builder"
kind = "claude"
args = ["--model", "sonnet", "--dangerously-skip-permissions"]
produces = "build-report.md"
timeout_min = 30

[[stage]]
id = "review"
role = "reviewer"
kind = "codex"
args = []
produces = "review.md"
verdict = true                 # line 1 must be `VERDICT: APPROVE` or `VERDICT: CHANGES`
on_changes = "build"
max_iterations = 2
timeout_min = 10

[[stage]]
id = "tutorial"
role = "tutorial"
kind = "claude"
args = ["--model", "sonnet", "--dangerously-skip-permissions"]
produces = "tutorial.md"
timeout_min = 10
optional = true                # skip with --skip tutorial

[finish]
open_pr = true
pr_body_from = ["plan.md", "review.md"]
```

Validation (fail at load time with clear errors): unique stage ids; `on_changes` refers to an earlier stage; `verdict` stages have `max_iterations >= 1`; the prompt file for each role exists; `kind` is one of herdr's supported kinds; `produces` is a plain relative path inside the run dir.

### Prompt templates (`prompts/<role>.md`)

Strict `{{name}}` substitution. An unknown variable is an error, and so is a leftover `{{`. Variables:

`{{run_id}} {{run_dir}} {{brief_path}} {{produces}} {{iteration}} {{previous_review}} {{repo}} {{branch}} {{base_branch}} {{stage_id}}`

`{{previous_review}}` is a path to the last review artifact, or the literal `none`.

---

## CLI surface

```
muster init --out <dir> [--force]                  # write default config tree (for the Helm chart)
muster doctor                                      # check runtime contract
muster list-workflows
muster validate [<workflow>]                       # load + validate config, workflows, prompts
muster start <workflow> --repo <repo> (--brief-file <path> | --brief-stdin)
              [--run-id <id>] [--skip <stage>...] [--detach]
muster resume <run-id>
muster status [<run-id>]
muster clean <run-id> [--delete-branch]
```

Global flags: `--config-dir`, `--json`, `-v`.

`muster validate --config-dir <path>` needs no herdr, git or network access. The templates repo can run it in CI against the chart's config files before ArgoCD syncs them.

Exit codes: 0 done; 2 usage or config error; 3 run needs a human (escalated); 4 run failed (unrecoverable); 5 concurrency limit reached.

`--detach` re-execs itself in the background, logs to `<log_dir>/<run>.log`, prints the run id, and exits 0. Wrappers need this to start runs without blocking.

Run ids: auto-generated as `<workflow letter><n>`, for example `f12` or `b3`. They must satisfy herdr's agent-name rule `[a-z][a-z0-9_-]{0,31}` when combined with the role, as in `build-f12`.

---

## Phase 0: Repo bootstrap

1. `cargo new muster`, set up the layout, and add placeholder modules.
2. Write `CLAUDE.md` for this repo: build with `cargo build`; verify with `cargo test` and `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`; every herdr call goes through `herdr.rs`; never commit secrets.
3. Start `docs/DECISIONS.md`.

**Acceptance:** `muster --help` lists every subcommand, each returning "not implemented" with exit code 2. CI-style checks pass.

## Phase 1: Config, workflow, and prompt loading

Implement `config.rs`, `workflow.rs`, `render.rs`, plus `init`, `list-workflows`, and `validate`. Write the default files in `defaults/`. Phase 6 refines the prompt contents; for now the prompts only need to exercise every variable.

**Acceptance:** unit tests cover config-dir lookup order, `~` expansion, every validation error, rendering (including unknown-variable and leftover-brace errors), and `--skip` of an optional stage. A test builds a ConfigMap-style directory (files as symlinks into `..data`, plus a stale `..2026_...` dir) and confirms that loading follows the symlinks and ignores the dot entries. `muster init --out /tmp/cfg && muster validate --config-dir /tmp/cfg` succeeds. Validation works with a read-only config dir.

## Phase 2: herdr client behind a trait

`herdr.rs` exposes a `Herdr` trait with a real implementation that shells out, and a recorded or fake implementation for tests. Methods (each maps to one CLI call; **verify the flags against the binary**):

| Method | herdr CLI |
|---|---|
| `status()` | `herdr status server` |
| `worktree_create(repo_path, branch, label)` | `herdr worktree create --cwd <path> --branch <b> --label <l> --no-focus` → workspace id, root pane id |
| `worktree_remove(workspace_id, force)` | `herdr worktree remove --workspace <id> [--force]` |
| `pane_split(pane, direction)` | `herdr pane split <pane> --direction right\|down --no-focus` → new pane id |
| `pane_get(pane)` | `herdr pane get <pane>` |
| `agent_start(name, kind, pane, args, timeout)` | `herdr agent start <name> --kind <kind> --pane <pane> --timeout <ms> -- <args...>` |
| `agent_prompt_wait(name, text, until, timeout)` | `herdr agent prompt <name> <text> --wait --until idle --until done --until blocked --timeout <ms>` |
| `agent_get(name)` | `herdr agent get <name>` |
| `agent_read(name, lines)` | `herdr agent read <name> --source recent --lines <n>` |
| `notify(title, body, sound)` | `herdr notification show <title> --body <body> --sound done\|request` |
| `report_metadata(pane, tokens)` | `herdr pane report-metadata <pane> --source muster --token k=v ...` |

All calls pass `--session <s>` when configured. A non-zero exit becomes a typed error that preserves the JSON on stderr: timeout, `agent_prompt_stalled`, `agent_not_ready`, `agent_blocked`, `not_found`, and other.

Build `tests/fake-herdr/`: a small Rust test binary or shell script that the real implementation can point to through `HERDR_BIN`. It reads a scenario file, returns canned JSON per command, can simulate an agent "writing" an artifact (by creating a file when prompted), and logs every invocation for assertions.

**Acceptance:** unit tests for argument construction and error mapping. One integration test drives the real `Herdr` implementation against fake-herdr.

## Phase 3: Manual dry run (HUMAN-guided)

Before writing the engine, run one workflow by hand against the real herdr server on the homelab pod, so the engine and its fixtures reflect real behavior in the actual environment.

**HUMAN:** create a throwaway GitHub repo `muster-sandbox`: a tiny Rust crate with one failing test, cloned into `repos_dir`.

Claude Code writes the steps into `docs/USAGE.md` under "Manual run"; the operator runs them and pastes back the outputs:

```sh
herdr worktree create --cwd ~/repos/muster-sandbox --branch muster/manual/m1 --label m1 --no-focus
herdr agent start plan-m1 --kind claude --pane <root_pane> -- --model opus --dangerously-skip-permissions
herdr agent prompt plan-m1 "Read .muster/runs/m1/prompts/plan-1.md and follow it exactly." --wait --until idle --until done --until blocked --timeout 600000
herdr agent read plan-m1 --source recent --lines 60
herdr pane split <root_pane> --direction right --no-focus
```

Record the following in `tests/fixtures/` (raw JSON plus stderr and exit code) and answer the questions in `DECISIONS.md`:
- What JSON do `worktree create`, `pane split`, `agent start`, and `agent prompt --wait` return, and with what exit codes, for done, blocked, timeout, and stalled?
- Does `agent start` hit `agent_not_ready` on a first-run trust or onboarding prompt? What arguments or setup avoid it?
- With permission prompts skipped, does the agent ever reach `blocked`?
- How does an agent exit cleanly so its pane returns to an idle shell (for example `/exit` through `agent prompt`, or `agent send-keys ctrl+c`)?

**Acceptance:** fixtures committed; `DECISIONS.md` answers every question; `herdr.rs` parsing updated to the real shapes.

## Phase 4: Run state and engine

`state.rs`: run state JSON at `<state_dir>/<run>.json`. It holds workflow name, repo, branch, worktree path, workspace id, a role-to-pane-and-agent map, current stage, iteration, status (`running | needs_human | done | failed`), the escalation reason, and a timestamped event log.

**Config snapshot:** at `start`, copy the workflow TOML and every prompt template it uses into `<state_dir>/<run>/config/`, and record a content hash of the snapshot in the run state. The rest of the run (including every `resume`) reads only from the snapshot, never the live config dir. Also copy the snapshot into the committed run dir (`.muster/runs/<run>/config/`) so the paper trail shows exactly which prompts produced the work. `status <run>` shows whether the live config has changed since the run started. Write atomically (temp file, then rename). Use a per-run lock file so two invocations can't drive the same run. Count active runs (`running`) for the concurrency limit.

`engine.rs`, the `start` algorithm:

1. Check the concurrency limit (exit 5).
2. Run `git fetch` on the repo, then call `worktree_create` with branch `<branch_prefix>/<run-id>` from the base branch. **VERIFY** whether `herdr worktree create` accepts `--base`; if it does, use `origin/<base_branch>`.
3. Write `brief.md` into `.muster/runs/<run>/` and commit it (`muster: start <run>`).
4. For each stage, unless skipped:
   1. Get an idle pane for the role: reuse the role's pane if its agent has exited; otherwise split from the most recent pane, alternating right and down.
   2. If the role's agent isn't live, run `agent_start(<role>-<run>, kind, pane, args)`.
   3. Render the prompt to `.muster/runs/<run>/prompts/<stage>-<iter>.md`.
   4. `agent_prompt_wait(name, "Read <path> and follow it exactly.", timeout)`.
   5. Classify the outcome:
      - Error (timeout, stalled, not ready, blocked) → escalate. **Never retry.**
      - Settled, but the artifact is missing, empty, or contains a `## Blocked` heading → escalate.
      - Verdict stage: parse line 1. `APPROVE` → continue. `CHANGES` → if `iteration < max_iterations`, set `previous_review` and jump to `on_changes`; otherwise escalate.
      - Otherwise → next stage.
   6. Append an event to the log and call `report_metadata(pane, stage=<id>, iter=<n>)`.
5. Run the finish step (Phase 5).

**Escalate:** set status `needs_human` with the reason, call `notify("<run>: <stage> needs you", reason, request)`, print a resume hint, and exit 3.

**`resume`:** re-validate the run's config snapshot (not the live config), then continue from the recorded stage and iteration. If the role's agent is gone, restart it in its pane. Re-check the artifact first, since the human may have finished the stage by hand.

**`status`:** a table of runs (id, workflow, repo, stage, iteration, status, age), or the detail and event log for one run.

**`clean`:** close the run's panes, call `worktree_remove`, and optionally delete the branch. Refuse on a running run without `--force`.

**Acceptance:** fake-herdr integration tests cover the happy path; a review loop that approves on iteration 2; a loop that hits `max_iterations` and escalates; a missing artifact; a `## Blocked` artifact; a timeout; resume after escalation; the concurrency limit; `--skip`; and a config change between escalation and `resume` that has no effect on the resumed run.

## Phase 5: Finish step and paper trail

`git.rs`:
1. If the worktree is dirty, commit the leftovers as `muster: uncommitted changes after <stage>` and log a warning event, since agents are supposed to commit.
2. If Entire is enabled, record the output of `entire checkpoint list --outgoing` in the event log. **VERIFY** the flags.
3. `git push -u <remote> <branch>`.
4. `gh pr create --base <base> --head <branch> --title "<first line of brief>" --body-file <assembled>`. The body is the files listed in `pr_body_from`, plus a footer with the run id and the run dir path.
5. Set status `done` and call `notify("<run> ready", <PR url>, done)`.

Entire setup per worktree happens right after `worktree_create`: if `entire.enabled`, run `entire enable` non-interactively for the claude and codex agents (**VERIFY** the flags). If `checkpoint_remote` is set, run `entire configure --local --checkpoint-remote <remote>`. If `entire` is missing while enabled, `doctor` flags it and `start` warns once and continues.

**Acceptance:** integration test with a local bare git repo as the remote and a fake `gh` script (`GH_BIN` override) confirms the push and PR body assembly. A dirty worktree produces the leftover commit.

## Phase 6: Default workflows and prompts

Write production-quality versions of `defaults/workflows/*.toml` and `defaults/prompts/*.md`.

Every prompt states: the role; the inputs to read; the exact output file `{{produces}}`; the completion contract ("you are done when `{{produces}}` exists; do not ask questions; if you cannot proceed, write your question under a `## Blocked` heading in `{{produces}}` and stop"); and the commit rule.

- **planner:** reads the brief and repo, writes numbered implementation steps and a definition-of-done checklist to `plan.md`. Makes no code changes.
- **builder:** implements the plan step by step, committing after each checklist item. Runs the repo's checks and writes `build-report.md` (changes, check commands and their results, deviations from the plan). On iteration > 1, addresses every item in `{{previous_review}}`.
- **reviewer:** reviews `git diff {{base_branch}}...HEAD` against the plan and build report. Writes `review.md` with line 1 `VERDICT: APPROVE` or `VERDICT: CHANGES`, followed by concrete issues. Doesn't edit code.
- **tutorial:** writes `tutorial.md` explaining how the change works and how to verify it.
- **bugfix workflow:** reproducer (failing test plus `repro.md`), then fixer, then reviewer.
- **investigate workflow:** a single investigator stage producing `report.md`, with no PR (`open_pr = false`). It must not modify files outside the run dir. **VERIFY** the Claude Code flags for restricting tools.

`defaults/AGENTS.muster.md`, a snippet for target repos: commit per plan checklist item, referencing the item number; never rewrite history or push (the conductor pushes); only touch your own stage's artifact under `.muster/runs/`; preserve Entire's settings and hooks.

**Acceptance:** `muster validate` passes for all three workflows. A test renders every prompt with sample variables and leaves no unresolved placeholders.

## Phase 7: doctor, packaging, docs

- `muster doctor`: checks each item of the runtime contract (herdr binary and server status; installed integrations; `claude` and `codex` on PATH; `git`; `gh auth status`; `entire` if enabled; that the config dir exists and validates; that the data directories exist and are writable; that every configured repo is cloned) and prints pass or fail with fix hints. Supports `--json`. Exits non-zero if any check fails. The deployment can use it as a startup or readiness check.
- Release builds: `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` static binaries, plus macOS. Add a `justfile` with `build`, `test`, `lint`, and `release` recipes. The templates repo will consume these binaries.
- Docs: `docs/USAGE.md` (commands, config, writing a new workflow) and `docs/RUNTIME.md` (the contract above).

**Acceptance:** `muster doctor` on a machine without herdr reports clear failures. Release binaries build.

## Phase 8: End-to-end run (HUMAN)

On the homelab pod, after the templates repo has deployed the binary and the config directory, run this from any machine over SSH or inside a `herdr --remote muster` pane:

```sh
muster doctor
echo "Make the failing test pass and add a doc comment to the public fn" | muster start feature --repo muster-sandbox --brief-stdin
```

**Acceptance:** the run reaches `done` and opens a PR on `muster-sandbox`. The run dir contains every artifact and rendered prompt. With Entire enabled, the commits carry `Entire-Checkpoint` trailers. Forcing `timeout_min = 1` on build escalates, and `muster resume` completes the run.

---

## Gotchas (read before implementing)

- **`agent prompt --wait` does not track individual turns.** A settled state only means the agent stopped. Always verify the artifact exists.
- **Timeouts don't prove the prompt wasn't delivered.** Never retry automatically.
- **`agent start` requires an idle shell pane.** Create the topology first (split), then start the agent.
- **Agent names** must match `[a-z][a-z0-9_-]{0,31}` and be unique among live agents. Keep run ids short.
- **A herdr server restart stops pane processes.** Treat a vanished agent as needing a restart on `resume`, not as success.
- **The `idle` and `done` statuses both mean ready for input.** `done` is just "idle and not yet seen". Treat them the same.
- **Pane ids can change if panes move** between workspaces. Store agent names as the primary handle, and pane ids only for splitting and metadata.
- **The config dir is read-only and can change under you** whenever ArgoCD syncs. Never write to it, and never read from it after `start` (use the run's snapshot).

## Open questions for the operator

1. Codex or a second Claude model as the reviewer by default?
2. Is the `tutorial` stage on by default or opt-in?
3. Should PRs be opened as drafts?
4. GitHub owner for `muster-sandbox` and `muster-checkpoints`?

## References

- herdr CLI reference: https://herdr.dev/tui/docs/cli-reference/
- herdr socket API: https://herdr.dev/tui/docs/socket-api/
- herdr agent automation: https://herdr.dev/tui/docs/agent-automation/
- Entire CLI: https://github.com/entireio/cli
- Glass Town article: https://aaif.io/blog/welcome-to-glass-town-building-a-software-factory-with-a-paper-trail
