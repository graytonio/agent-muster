# Using muster

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
