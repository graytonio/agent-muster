<!-- Snippet for target repos: paste into AGENTS.md or CLAUDE.md. -->

## Working under muster

This repo is sometimes worked on by agents that muster runs as stages of a workflow (planner, builder, reviewer, ...). Each stage gets its instructions from a prompt file under `.muster/runs/<run>/prompts/`. When you are one of those agents:

- **Commit per plan item.** Make one commit for each checklist item or numbered step you complete, and reference it in the message, for example `step 3: add retry to fetch_config`. Commit only your code changes, with explicit paths.
- **Never rewrite history or push.** No `rebase`, `commit --amend`, `reset --hard`, `push` or `push --force`. muster pushes the branch and opens the PR when the run finishes.
- **Stay in your lane under `.muster/runs/`.** Write only your own stage's artifact (the file your prompt names). Don't edit the brief, other stages' artifacts, the rendered prompts or the `config/` snapshot. muster commits the run directory after each stage.
- **Don't ask questions.** Nobody is watching the pane. If you can't proceed, write your question under a `## Blocked` heading in your artifact and stop; a human will answer and resume the run.
- **Preserve Entire's setup.** If the repo has `.entire/` settings or git hooks installed by Entire, don't remove or modify them; they record session checkpoints for each commit.
