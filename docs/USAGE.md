# Using muster

muster runs a workflow (a sequence of agent stages) against a repo on a herdr server. Each stage is an agent in its own herdr pane. Stages hand off through artifact files in the run directory, and the run finishes by pushing the branch and opening a PR.

See `RUNTIME.md` for what the host must provide, and `DECISIONS.md` for why things work the way they do.

## Quick start

```sh
muster doctor                                   # is the host ready?
muster list-workflows
muster start feature --repo muster-sandbox --brief-file brief.md
echo "Fix the flaky retry test" | muster start bugfix --repo muster-sandbox --brief-stdin --detach
muster start bugfix --repo muster-sandbox --brief "Fix the flaky retry test in src/net.rs"
muster status                                   # all runs
muster status f1                                # one run, with its event log
muster resume f1                                # after a human fixed what it escalated
muster clean f1 --delete-branch                 # close panes, remove the worktree
```

From a laptop, over Tailscale SSH: `ssh muster 'muster start feature --repo foo --brief-stdin --detach' < brief.md`, or for a short brief `ssh muster 'muster start bugfix --repo foo --brief "Fix the flaky retry test" --detach'`.

## Commands

| Command | What it does |
|---|---|
| `init --out <dir> [--force]` | Write the default config tree. Refuses a non-empty dir without `--force`, and refuses the live config dir. |
| `doctor` | Check the runtime contract. Non-zero exit if anything fails. |
| `list-workflows` | Workflows in the config dir, with their stages. |
| `validate [<workflow>]` | Load and validate config, workflows and prompts. No herdr, git or network. |
| `start <workflow> --repo <repo> (--brief <text> \| --brief-file <path> \| --brief-stdin)` | Start a run. Exactly one brief source is required (write `--brief=-x` for text starting with `-`). `--run-id <id>` to choose the id, `--skip <stage>` (repeatable) for optional stages, `--detach` to run in the background. |
| `resume <run>` | Continue a run from its recorded stage, using its config snapshot. |
| `status [<run>]` | A table of runs, or one run's detail and event log. Shows whether the live config changed since the run started. |
| `clean <run> [--delete-branch] [--force]` | Close the run's panes and remove its worktree. `--force` cleans a running run and discards uncommitted changes. |

Global flags: `--config-dir <path>`, `--json` (every command prints one JSON object on stdout), `-v` (log every herdr/git/gh call to stderr).

Exit codes: `0` done, `2` usage or config error, `3` the run needs a human, `4` the run failed, `5` concurrency limit reached.

Environment: `MUSTER_CONFIG_DIR`, `HERDR_BIN`, `GH_BIN`, `ENTIRE_BIN`.

## What a run does

1. Checks the concurrency limit, picks a run id (`f1`, `b3`, ...: the workflow's first letter and a number), and snapshots the workflow and its prompts into `<state_dir>/<run>/config/`.
2. `git fetch`, then `herdr worktree create` on branch `<branch_prefix>/<run>` from `<remote>/<base_branch>`.
3. Writes `.muster/runs/<run>/brief.md` and a copy of the snapshot, and commits them (`muster: start <run>`).
4. For each stage: starts the role's agent (`<role>-<run>`) in a pane if it isn't running, renders the prompt to `.muster/runs/<run>/prompts/<stage>-<iteration>.md`, sends "Read <that file> and follow it exactly.", and waits for the agent to settle. Then it checks the artifact:
   - missing, empty, or containing a `## Blocked` heading → **escalate**
   - a `read_only` stage changed files outside the run dir → **escalate**
   - a `verdict` stage: line 1 `VERDICT: APPROVE` → continue; `VERDICT: CHANGES` → loop back to `on_changes` with the next iteration, or **escalate** once `max_iterations` is reached
   - any herdr error (timeout, stalled, not ready, blocked) → **escalate**. Nothing is ever retried automatically.

   After each stage muster commits the run dir (`muster: <stage> iteration <n> for <run>`).
5. Finish: commits anything the agents left uncommitted (with a warning event), records Entire's outgoing checkpoints, pushes, and opens the PR with `gh pr create`. The PR title is the brief's first line; the body is the `pr_body_from` artifacts plus a footer.

**Escalation** sets the run to `needs_human`, sends a herdr notification, prints the reason and a resume hint, and exits 3.

## Handling an escalated run

`muster status <run>` shows the reason and the event log. Then, depending on the cause:

- **The agent asked a question (`## Blocked`)**: write the answer in `.muster/runs/<run>/human-notes.md` and `muster resume <run>`. The stage is prompted again and told to read the notes. Or finish the artifact yourself and remove the `## Blocked` section; resume will accept it without prompting.
- **The artifact is missing**: look at the agent (`herdr agent read <role>-<run>`). Either write the artifact by hand, or just resume to prompt the stage again.
- **Timeout**: the prompt may still have been delivered and the agent may still be working. Check the agent first. If it finished and wrote the artifact, resume accepts it. Raise `timeout_min` in the workflow for future runs.
- **The review loop hit `max_iterations`**: read the last `review.md`, fix the code yourself or decide to accept it, rewrite line 1 of `review.md` to `VERDICT: APPROVE`, and resume.
- **A read-only stage changed files**: revert those changes, then resume.
- **Finish failed (push or PR)**: fix the cause (auth, remote) and resume; it retries only the finish step.

Resume reads the run's config snapshot, never the live config. If herdr restarted, the agents are gone; resume starts them again in their panes.

## Configuration

`muster.toml` (see `defaults/muster.toml` for the commented version):

```toml
repos_dir = "~/repos"
state_dir = "~/.muster/state"
log_dir   = "~/.muster/logs"
worktrees_dir = ""             # empty: herdr's default
max_concurrent_runs = 2

[herdr]
bin = "herdr"
session = ""

[git]
remote = "origin"
base_branch = "main"

[entire]
enabled = false
agents = ["claude-code"]
checkpoint_remote = ""         # e.g. "github:<owner>/muster-checkpoints"

[repos.muster-sandbox]
base_branch = "main"
```

## Writing a new workflow

Add `workflows/<name>.toml` (the file name must match `workflow.name`) and any new `prompts/<role>.md` to the config dir (in practice, the Helm chart), then run `muster validate <name>`. No code changes are needed.

```toml
[workflow]
name = "docs"                  # [a-z][a-z0-9_-]*, must match the file name
description = "Write docs for a module"
branch_prefix = "muster/docs"  # branches are <branch_prefix>/<run>

[[stage]]
id = "draft"                   # unique within the workflow
role = "writer"                # prompts/writer.md; agent name writer-<run>
kind = "claude"                # a herdr agent kind
args = ["--model", "sonnet", "--dangerously-skip-permissions"]
produces = "draft.md"          # artifact, relative to the run dir
timeout_min = 20               # default 30

[[stage]]
id = "edit"
role = "editor"
kind = "claude"
args = ["--model", "opus", "--dangerously-skip-permissions"]
produces = "edit-review.md"
verdict = true                 # line 1: VERDICT: APPROVE | VERDICT: CHANGES
on_changes = "draft"           # an earlier, non-optional stage
max_iterations = 2             # stages on the loop run at most this many times
read_only = true               # escalate if it changes files outside the run dir

[[stage]]
id = "summary"
role = "summarizer"
kind = "claude"
produces = "summary.md"
optional = true                # can be skipped with --skip summary

[finish]
open_pr = true                 # false: no push, no PR; artifacts stay on the local branch
draft = false
pr_body_from = ["draft.md", "edit-review.md"]   # must be produced by some stage
```

Prompt templates use strict `{{name}}` substitution; an unknown variable or a stray `{{` is a validation error. Variables: `{{run_id}}`, `{{run_dir}}`, `{{brief_path}}`, `{{produces}}`, `{{iteration}}`, `{{previous_review}}` (path to the last review artifact, or `none`), `{{repo}}`, `{{branch}}`, `{{base_branch}}`, `{{stage_id}}`. Paths are absolute.

Every prompt should state the role, the inputs to read, the exact output file (`{{produces}}`), the completion contract ("you are done when `{{produces}}` exists; if you cannot proceed, write your question under a `## Blocked` heading in it and stop"), and the commit rule. `defaults/AGENTS.muster.md` is a snippet for target repos' `AGENTS.md`/`CLAUDE.md` with the same rules.

## The paper trail

Everything a run did is committed on its branch under `.muster/runs/<run>/`:

```
brief.md
config/workflows/<workflow>.toml   # the snapshot this run used
config/prompts/<role>.md
prompts/<stage>-<iteration>.md     # exactly what each agent was told
<artifacts>                        # plan.md, build-report.md, review.md, ...
<artifact>-<k>.md                  # earlier iterations' artifacts
human-notes.md                     # if a human left notes
```

The run state and event log live in `<state_dir>/<run>.json` (not committed).

## Manual run (Phase 3)

This walks through one stage by hand against a real herdr server, to record what herdr actually returns when a live agent is involved. Run it on the homelab pod (or any machine with herdr and an authenticated `claude`), in a **named session** so it can't disturb the operator's panes.

The non-agent responses (`worktree create`, `pane split`, `pane get`, error codes) are already recorded in `tests/fixtures/herdr/`. What's missing is anything that needs a live agent.

Set up:

```sh
# An isolated herdr server for the experiment (leave it running in its own terminal).
herdr --session mdry server

# In another terminal. A throwaway clone of muster-sandbox (or any small repo).
export R=~/repos/muster-sandbox
export FX=~/muster-fixtures && mkdir -p "$FX"
rec() { name=$1; shift; herdr --session mdry "$@" >"$FX/$name.stdout" 2>"$FX/$name.stderr"; echo $? >"$FX/$name.exit"; echo "--- $name exit=$(cat "$FX/$name.exit")"; cat "$FX/$name.stdout" "$FX/$name.stderr"; }
git -C "$R" fetch origin
```

1. Create the worktree and note `root_pane` and `worktree.path` from the output:

   ```sh
   rec worktree_create worktree create --cwd "$R" --branch muster/manual/m1 --base origin/main --label m1 --no-focus
   export P=<root_pane>  W=<worktree.path>
   mkdir -p "$W/.muster/runs/m1/prompts"
   printf 'You are the planner. Read src/lib.rs and write a three-line plan for making the failing test pass to `.muster/runs/m1/plan.md`. Change no code. Do not ask questions. You are done when the file exists.\n' > "$W/.muster/runs/m1/prompts/plan-1.md"
   ```

2. Start the agent. **Question: does this return `agent_not_ready` on a first-run trust or onboarding prompt?** If it does, record `agent read` output too, accept the prompt with `herdr --session mdry agent send-keys plan-m1 enter` (or whatever the screen asks for), and note what setup would avoid it.

   ```sh
   rec agent_start agent start plan-m1 --kind claude --pane "$P" --timeout 60000 -- --model sonnet --dangerously-skip-permissions
   rec agent_read_after_start agent read plan-m1 --source recent-unwrapped --lines 60
   ```

3. Prompt and wait (the done case):

   ```sh
   rec agent_prompt_done agent prompt plan-m1 "Read $W/.muster/runs/m1/prompts/plan-1.md and follow it exactly." --wait --timeout 600000
   ls -l "$W/.muster/runs/m1/plan.md"
   rec agent_get agent get plan-m1
   ```

4. The timeout case: a prompt that takes longer than the timeout.

   ```sh
   rec agent_prompt_timeout agent prompt plan-m1 "Count slowly from 1 to 200, one number per line, then stop." --wait --timeout 5000
   herdr --session mdry agent wait plan-m1 --timeout 300000   # let it finish before the next step
   ```

5. The stalled case: prompting an agent that produces no activity. One way: send an empty-ish prompt such as `" "`.

   ```sh
   rec agent_prompt_stalled agent prompt plan-m1 " " --wait --timeout 30000
   ```

6. **Question: with permission prompts skipped, does the agent ever reach `blocked`?** To see what `blocked` looks like, start a second agent *without* `--dangerously-skip-permissions` in a split pane and ask it to write a file:

   ```sh
   rec pane_split2 pane split "$P" --direction right --no-focus      # note the new pane id as P2
   rec agent_start_perm agent start perm-m1 --kind claude --pane "$P2" --timeout 60000 -- --model sonnet
   rec agent_prompt_blocked agent prompt perm-m1 "Create the file $W/x.txt containing hi." --wait --timeout 120000
   rec agent_prompt_while_blocked agent prompt perm-m1 "hello" --wait --timeout 5000
   ```

7. **Question: how does an agent exit cleanly so its pane is an idle shell again?** Try `/exit` first, then check the pane:

   ```sh
   rec agent_exit agent prompt plan-m1 "/exit"
   rec pane_get_after_exit pane get "$P"
   rec agent_get_after_exit agent get plan-m1
   # If the agent is still there, try: herdr --session mdry agent send-keys plan-m1 ctrl+c ctrl+c
   ```

8. Clean up:

   ```sh
   herdr --session mdry worktree remove --workspace <workspace_id> --force
   git -C "$R" branch -D muster/manual/m1
   herdr session stop mdry
   ```

Send back the `$FX` directory (or paste its files) and the answers to the four questions. They go in `tests/fixtures/herdr/` and `docs/DECISIONS.md`, and the engine's outcome handling gets checked against them.
