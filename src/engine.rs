//! The stage loop: start, resume, status and clean.
//!
//! The conductor is deterministic and sequential. It never retries a prompt:
//! any failure escalates to a human (`needs_human`, exit 3), who fixes things
//! and runs `muster resume`.

use std::collections::BTreeSet;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::config::{check_repo_name, Config};
use crate::error::{exit, MusterError};
use crate::git::{which, Git, MergeOutcome};
use crate::herdr::{Direction, ErrorKind, Herdr, HerdrError, Sound, WorktreeRequest};
use crate::render::{self, Vars};
use crate::state::{
    self, check_run_id, now, run_dir_rel, AgentSlot, Cursor, RunLock, RunState, RunStatus,
    StateStore, STATE_VERSION,
};
use crate::workflow::{self, Loaded, Stage};

/// How long `agent start` may take before herdr gives up.
pub const AGENT_START_TIMEOUT_MS: u64 = 60_000;
/// Optional file a human can leave in the run dir before `resume`; it is
/// mentioned to the agent when its stage is prompted again.
pub const HUMAN_NOTES: &str = "human-notes.md";

/// Result of driving a run, for printing.
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub run_id: String,
    pub status: RunStatus,
    pub reason: Option<String>,
    pub pr_url: Option<String>,
    pub branch: String,
    pub worktree: Option<PathBuf>,
    pub run_dir: Option<PathBuf>,
}

impl Outcome {
    pub fn exit_code(&self) -> i32 {
        match self.status {
            RunStatus::Done => exit::OK,
            RunStatus::NeedsHuman => exit::NEEDS_HUMAN,
            RunStatus::Failed => exit::FAILED,
            // Only seen when a run was handed to a detached process.
            RunStatus::Running => exit::OK,
        }
    }

    fn from_state(s: &RunState) -> Self {
        Self {
            run_id: s.id.clone(),
            status: s.status,
            reason: s.reason.clone(),
            pr_url: s.pr_url.clone(),
            branch: s.branch.clone(),
            worktree: s.worktree_path.clone(),
            run_dir: s.run_dir(),
        }
    }
}

/// Where the brief comes from.
pub enum Brief {
    File(PathBuf),
    Stdin,
    Text(String),
}

impl Brief {
    pub fn read(self) -> Result<String> {
        let text = match self {
            Brief::File(p) => std::fs::read_to_string(&p).map_err(|e| {
                MusterError::usage(format!("cannot read brief {}: {e}", p.display()))
            })?,
            Brief::Stdin => {
                let mut s = String::new();
                std::io::stdin()
                    .read_to_string(&mut s)
                    .context("reading brief from stdin")?;
                s
            }
            Brief::Text(t) => t,
        };
        if text.trim().is_empty() {
            bail!(MusterError::usage("the brief is empty"));
        }
        Ok(text)
    }
}

pub struct StartRequest {
    pub workflow: String,
    pub repo: String,
    pub brief: String,
    pub run_id: Option<String>,
    pub skip: Vec<String>,
}

/// Classification of a settled stage.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StageResult {
    Complete,
    Changes,
    Escalate(String),
}

pub struct Engine<'a> {
    pub cfg: &'a Config,
    pub herdr: &'a dyn Herdr,
    pub git: Git,
    pub store: StateStore,
    /// Print progress lines to stderr.
    pub progress: bool,
}

impl<'a> Engine<'a> {
    pub fn new(cfg: &'a Config, herdr: &'a dyn Herdr, verbose: bool, progress: bool) -> Self {
        Self {
            cfg,
            herdr,
            git: Git::new(cfg.gh_bin.clone(), verbose),
            store: StateStore::new(cfg.state_dir.clone()),
            progress,
        }
    }

    fn say(&self, run: &str, msg: impl AsRef<str>) {
        if self.progress {
            eprintln!("[{run}] {}", msg.as_ref());
        }
    }

    fn save(&self, s: &mut RunState) -> Result<()> {
        self.store.save(s)
    }

    // ---------------------------------------------------------------- start

    /// Reserve a run: check the concurrency limit, pick the id, snapshot the
    /// config and write the initial state. Returns the state and its lock.
    pub fn reserve(&self, req: &StartRequest) -> Result<(RunState, Loaded, RunLock)> {
        check_repo_name(&req.repo)?;
        let loaded = workflow::load(&self.cfg.config_dir, &req.workflow)?;
        let skip = loaded.workflow.resolve_skips(&req.skip)?;
        let repo_path = self.cfg.repo_path(&req.repo);
        if !repo_path.join(".git").exists() {
            bail!(MusterError::usage(format!(
                "repo {} is not cloned at {}",
                req.repo,
                repo_path.display()
            )));
        }

        let _start = self.store.start_lock()?;
        self.check_concurrency(None)?;
        let id = match &req.run_id {
            Some(id) => {
                check_run_id(id)?;
                if self.store.exists(id) {
                    bail!(MusterError::usage(format!("run {id} already exists")));
                }
                id.clone()
            }
            None => self.store.next_run_id(&req.workflow)?,
        };
        for role in loaded.workflow.roles() {
            let name = format!("{role}-{id}");
            if name.len() > 32 {
                bail!(MusterError::usage(format!(
                    "agent name {name} is longer than herdr's 32-character limit; use a shorter --run-id"
                )));
            }
        }

        // Snapshot the config this run will use for its whole life.
        let files = state::snapshot_files(&loaded);
        let snap = self.store.snapshot_dir(&id);
        state::write_files(&snap, &files)?;
        let lock = self.store.lock(&id)?;

        let base_branch = self.cfg.base_branch_for(&req.repo);
        let first = loaded.workflow.stages.first().map(|s| s.id.clone());
        let ts = now();
        let mut st = RunState {
            version: STATE_VERSION,
            id: id.clone(),
            workflow: req.workflow.clone(),
            repo: req.repo.clone(),
            repo_path,
            branch: format!("{}/{id}", loaded.workflow.workflow.branch_prefix),
            base_branch,
            remote: self.cfg.git.remote.clone(),
            worktree_path: None,
            workspace_id: None,
            root_pane: None,
            agents: Default::default(),
            last_pane: None,
            splits: 0,
            skip,
            cursor: Cursor {
                stage: first,
                iteration: 1,
                prompted: false,
                stage_start_head: None,
                stage_start_dirty: Vec::new(),
            },
            previous_review: None,
            status: RunStatus::Running,
            reason: None,
            config_hash: state::hash_files(&files),
            pr_url: None,
            pid: Some(std::process::id()),
            created_at: ts.clone(),
            updated_at: ts,
            cleaned_at: None,
            events: Vec::new(),
        };
        st.event(
            "created",
            None,
            format!("workflow {} on {}", req.workflow, req.repo),
            Some(json!({"config_hash": st.config_hash, "skip": st.skip})),
        );
        self.save(&mut st)?;
        // Reload the workflow from the snapshot so the run never reads the
        // live config again.
        let loaded = workflow::load(&snap, &req.workflow)?;
        Ok((st, loaded, lock))
    }

    pub fn start(&self, req: StartRequest) -> Result<Outcome> {
        let (mut st, loaded, _lock) = self.reserve(&req)?;
        self.say(
            &st.id,
            format!("started {} on {} ({})", st.workflow, st.repo, st.branch),
        );
        if let Err(e) = self.setup(&mut st, &req.brief) {
            let msg = format!("{e:#}");
            st.status = RunStatus::Failed;
            st.reason = Some(format!("setup failed: {msg}"));
            st.event("failed", None, msg, None);
            self.save(&mut st)?;
            self.say(&st.id, st.reason.clone().unwrap_or_default());
            return Ok(Outcome::from_state(&st));
        }
        self.drive(&mut st, &loaded)
    }

    /// Fetch, create the worktree, set up Entire, write and commit the brief.
    fn setup(&self, st: &mut RunState, brief: &str) -> Result<()> {
        self.git.fetch(&st.repo_path, &st.remote)?;
        let base = format!("{}/{}", st.remote, st.base_branch);
        let path = self
            .cfg
            .worktrees_dir
            .as_ref()
            .map(|d| d.join(&st.repo).join(&st.id));
        let wt = self
            .herdr
            .worktree_create(&WorktreeRequest {
                repo_path: &st.repo_path,
                branch: &st.branch,
                base: Some(&base),
                path: path.as_deref(),
                label: &st.id,
            })
            .map_err(|e| anyhow::anyhow!("worktree create: {e}"))?;
        st.worktree_path = Some(wt.path.clone());
        st.workspace_id = Some(wt.workspace_id.clone());
        st.root_pane = Some(wt.root_pane.clone());
        st.event(
            "worktree",
            None,
            format!("{} (workspace {})", wt.path.display(), wt.workspace_id),
            None,
        );
        self.save(st)?;

        if self.cfg.entire.enabled {
            if which(&self.git.entire_bin).is_none() {
                st.event(
                    "warning",
                    None,
                    "entire is enabled but not on PATH; continuing without checkpoints",
                    None,
                );
            } else if let Err(e) = self.git.entire_setup(
                &wt.path,
                &self.cfg.entire.agents,
                &self.cfg.entire.checkpoint_remote,
            ) {
                st.event("warning", None, format!("entire setup failed: {e:#}"), None);
            } else {
                st.event("entire", None, "entire enabled for this worktree", None);
            }
        }

        let run_dir = wt.path.join(run_dir_rel(&st.id));
        std::fs::create_dir_all(run_dir.join("prompts"))?;
        std::fs::write(run_dir.join("brief.md"), brief)?;
        // The committed copy of the snapshot: the trail shows which prompts
        // produced the work.
        copy_dir(&self.store.snapshot_dir(&st.id), &run_dir.join("config"))?;
        self.git.commit_path(
            &wt.path,
            &run_dir_rel(&st.id),
            &format!("muster: start {}", st.id),
        )?;
        st.event("brief", None, "brief and config snapshot committed", None);
        self.save(st)
    }

    // --------------------------------------------------------------- resume

    /// The up-front checks of a resume, with no changes to the run: id,
    /// existence, lock, cleaned and concurrency. Returns the held lock and the
    /// loaded state. A `Done` run passes so the caller can report it.
    pub fn resume_preflight(&self, id: &str) -> Result<(RunLock, RunState)> {
        check_run_id(id)?;
        if !self.store.exists(id) {
            bail!(MusterError::usage(format!("no run named {id}")));
        }
        let lock = self.store.lock(id)?;
        let st = self.store.load(id)?;
        if st.status != RunStatus::Done {
            if st.cleaned_at.is_some() {
                bail!(MusterError::usage(format!(
                    "run {id} was cleaned; it cannot be resumed"
                )));
            }
            let _start = self.store.start_lock()?;
            self.check_concurrency(Some(id))?;
        }
        Ok((lock, st))
    }

    /// Fail with the concurrency exit code when the limit is reached. The
    /// caller holds the start lock.
    fn check_concurrency(&self, except: Option<&str>) -> Result<()> {
        let active = self.store.active(except)?;
        if active.len() as u32 >= self.cfg.max_concurrent_runs {
            bail!(MusterError::new(
                exit::CONCURRENCY,
                format!(
                    "concurrency limit reached: {} of {} runs active ({})",
                    active.len(),
                    self.cfg.max_concurrent_runs,
                    active.join(", ")
                )
            ));
        }
        Ok(())
    }

    pub fn resume(&self, id: &str) -> Result<Outcome> {
        let (lock, mut st) = self.resume_preflight(id)?;
        if st.status == RunStatus::Done {
            self.say(id, "already done");
            return Ok(Outcome::from_state(&st));
        }
        {
            let _start = self.store.start_lock()?;
            self.check_concurrency(Some(id))?;
            st.status = RunStatus::Running;
            st.reason = None;
            st.pid = Some(std::process::id());
            st.event(
                "resumed",
                st.cursor.stage.clone().as_deref(),
                "resumed by a human",
                None,
            );
            self.save(&mut st)?;
        }
        // Re-validate the snapshot, never the live config.
        let loaded = workflow::load(&self.store.snapshot_dir(id), &st.workflow)
            .context("the run's config snapshot is invalid")?;

        if st.worktree_path.is_none() {
            // Setup never finished; the brief is gone with the failed start.
            bail!(MusterError::failed(format!(
                "run {id} failed before its worktree was created; clean it and start again"
            )));
        }
        let outcome = self.drive(&mut st, &loaded);
        drop(lock);
        outcome
    }

    // ---------------------------------------------------------------- drive

    fn drive(&self, st: &mut RunState, loaded: &Loaded) -> Result<Outcome> {
        let wf = &loaded.workflow;
        while let Some(stage_id) = st.cursor.stage.clone() {
            let idx = wf
                .stage_index(&stage_id)
                .with_context(|| format!("stage {stage_id} is not in the snapshot"))?;
            let stage = &wf.stages[idx];

            if st.skip.contains(&stage.id) {
                st.event("skipped", Some(&stage.id), "skipped with --skip", None);
                self.advance(st, wf, idx);
                self.save(st)?;
                continue;
            }

            let result = if st.cursor.prompted {
                // Resuming a stage whose prompt may have been delivered: the
                // human may have finished it by hand.
                match self.check_artifact(st, stage)? {
                    StageResult::Escalate(reason) if is_redo(&reason) => {
                        self.say(&st.id, format!("{}: {reason}; prompting again", stage.id));
                        st.event("redo", Some(&stage.id), reason, None);
                        st.cursor.prompted = false;
                        self.run_stage(st, loaded, stage)?
                    }
                    r => {
                        st.event(
                            "recheck",
                            Some(&stage.id),
                            format!("artifact re-checked on resume: {r:?}"),
                            None,
                        );
                        r
                    }
                }
            } else {
                self.run_stage(st, loaded, stage)?
            };

            match result {
                StageResult::Complete => {
                    self.stage_done(st, stage, "complete")?;
                    self.advance(st, wf, idx);
                }
                StageResult::Changes => {
                    let max = stage.max_iterations.unwrap_or(1);
                    if st.cursor.iteration >= max {
                        self.stage_done(st, stage, "changes requested")?;
                        return self.escalate(
                            st,
                            &stage.id,
                            format!(
                                "{} requested changes after {} iteration(s) (max_iterations = {max})",
                                stage.id, st.cursor.iteration
                            ),
                        );
                    }
                    self.stage_done(st, stage, "changes requested")?;
                    let target = stage.on_changes.clone().unwrap_or_default();
                    st.previous_review = Some(self.artifact_path(st, stage));
                    st.cursor = Cursor {
                        stage: Some(target.clone()),
                        iteration: st.cursor.iteration + 1,
                        prompted: false,
                        stage_start_head: None,
                        stage_start_dirty: Vec::new(),
                    };
                    st.event(
                        "loop",
                        Some(&stage.id),
                        format!(
                            "looping back to {target} (iteration {})",
                            st.cursor.iteration
                        ),
                        None,
                    );
                    self.say(
                        &st.id,
                        format!("{}: changes requested, back to {target}", stage.id),
                    );
                }
                StageResult::Escalate(reason) => {
                    return self.escalate(st, &stage.id, reason);
                }
            }
            self.save(st)?;
        }
        self.finish(st, loaded)
    }

    fn advance(&self, st: &mut RunState, wf: &workflow::Workflow, idx: usize) {
        st.cursor = Cursor {
            stage: wf.stages.get(idx + 1).map(|s| s.id.clone()),
            iteration: st.cursor.iteration,
            prompted: false,
            stage_start_head: None,
            stage_start_dirty: Vec::new(),
        };
    }

    /// Commit the run dir and report pane metadata after a stage settles.
    fn stage_done(&self, st: &mut RunState, stage: &Stage, what: &str) -> Result<()> {
        let wt = st.worktree_path.clone().context("no worktree")?;
        self.git.commit_path(
            &wt,
            &run_dir_rel(&st.id),
            &format!(
                "muster: {} iteration {} for {}",
                stage.id, st.cursor.iteration, st.id
            ),
        )?;
        if let Some(slot) = st.agents.get(&stage.role) {
            let tokens = [
                ("run", st.id.clone()),
                ("stage", stage.id.clone()),
                ("iter", st.cursor.iteration.to_string()),
            ];
            if let Err(e) = self.herdr.report_metadata(&slot.pane_id, &tokens) {
                st.event(
                    "warning",
                    Some(&stage.id),
                    format!("report-metadata: {e}"),
                    None,
                );
            }
        }
        st.event("stage_done", Some(&stage.id), what, None);
        self.say(&st.id, format!("{}: {what}", stage.id));
        Ok(())
    }

    fn artifact_path(&self, st: &RunState, stage: &Stage) -> PathBuf {
        st.run_dir().unwrap_or_default().join(&stage.produces)
    }

    /// Start the agent if needed, render the prompt, send it and classify.
    fn run_stage(&self, st: &mut RunState, loaded: &Loaded, stage: &Stage) -> Result<StageResult> {
        let wt = st.worktree_path.clone().context("no worktree")?;
        let run_dir = st.run_dir().context("no run dir")?;
        self.say(
            &st.id,
            format!("{}: iteration {}", stage.id, st.cursor.iteration),
        );

        // A previous iteration's artifact must not satisfy this one.
        let artifact = self.artifact_path(st, stage);
        if artifact.exists() {
            let archived = archive(&artifact)?;
            if st.previous_review.as_deref() == Some(artifact.as_path()) {
                st.previous_review = Some(archived.clone());
            }
            st.event(
                "archived",
                Some(&stage.id),
                format!("previous artifact moved to {}", rel_to(&archived, &wt)),
                None,
            );
        }

        let name = match self.ensure_agent(st, stage) {
            Ok(n) => n,
            Err(e) => {
                let reason = describe_herdr(&e, "agent start");
                st.event("error", Some(&stage.id), &reason, Some(raw(&e)));
                return Ok(StageResult::Escalate(reason));
            }
        };

        let vars = self.vars(st, stage, &run_dir);
        let template = loaded
            .prompts
            .get(&stage.role)
            .with_context(|| format!("no prompt for role {}", stage.role))?;
        let text = render::render(template, &vars)
            .map_err(|e| MusterError::config(format!("prompts/{}.md: {e}", stage.role)))?;
        let prompt_path = run_dir
            .join("prompts")
            .join(format!("{}-{}.md", stage.id, st.cursor.iteration));
        std::fs::create_dir_all(prompt_path.parent().unwrap())?;
        std::fs::write(&prompt_path, text)?;

        let mut message = format!("Read {} and follow it exactly.", prompt_path.display());
        let notes = run_dir.join(HUMAN_NOTES);
        if notes.is_file() {
            message.push_str(&format!(
                " A human left notes for this run in {}; read them first.",
                notes.display()
            ));
        }

        st.cursor.stage_start_head = Some(self.git.head(&wt)?);
        st.cursor.stage_start_dirty = self.git.dirty_paths(&wt)?;
        st.cursor.prompted = true;
        st.event(
            "prompt",
            Some(&stage.id),
            format!("prompted {name} with {}", rel_to(&prompt_path, &wt)),
            None,
        );
        // Saved before sending: a timeout doesn't prove the prompt wasn't
        // delivered, so resume must know it may have been.
        self.save(st)?;

        let timeout_ms = stage.timeout_min * 60_000;
        match self.herdr.agent_prompt_wait(&name, &message, timeout_ms) {
            Err(e) => {
                let reason = describe_herdr(&e, "agent prompt");
                st.event("error", Some(&stage.id), &reason, Some(raw(&e)));
                Ok(StageResult::Escalate(reason))
            }
            Ok(info) if !info.is_settled() => {
                let reason = format!(
                    "{name} stopped in state `{}` (blocked on a dialog or not recognized); inspect it with `herdr agent read {name}`",
                    info.agent_status
                );
                st.event("error", Some(&stage.id), &reason, None);
                Ok(StageResult::Escalate(reason))
            }
            Ok(_) => self.check_artifact(st, stage),
        }
    }

    fn vars(&self, st: &RunState, stage: &Stage, run_dir: &Path) -> Vars {
        let mut v = Vars::new();
        v.insert("run_id", st.id.clone());
        v.insert("run_dir", run_dir.display().to_string());
        v.insert("brief_path", run_dir.join("brief.md").display().to_string());
        v.insert(
            "produces",
            run_dir.join(&stage.produces).display().to_string(),
        );
        v.insert("iteration", st.cursor.iteration.to_string());
        v.insert(
            "previous_review",
            st.previous_review
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "none".into()),
        );
        v.insert("repo", st.repo.clone());
        v.insert("branch", st.branch.clone());
        v.insert("base_branch", st.base_branch.clone());
        v.insert("stage_id", stage.id.clone());
        v
    }

    /// Make sure the role's agent is live and return its name.
    fn ensure_agent(&self, st: &mut RunState, stage: &Stage) -> Result<String, HerdrError> {
        let name = format!("{}-{}", stage.role, st.id);
        if let Some(info) = self.herdr.agent_get(&name)? {
            if let Some(slot) = st.agents.get_mut(&stage.role) {
                slot.pane_id = info.pane_id.clone();
            }
            return Ok(name);
        }
        // Not live: reuse the role's pane if it still exists, else the root
        // pane for the first agent, else split the most recent pane.
        let mut candidates = Vec::new();
        if let Some(slot) = st.agents.get(&stage.role) {
            candidates.push(slot.pane_id.clone());
        }
        if st.agents.is_empty() {
            if let Some(root) = &st.root_pane {
                candidates.push(root.clone());
            }
        }
        for pane in candidates {
            if self.herdr.pane_get(&pane).is_err() {
                continue;
            }
            match self.start_agent_in(st, stage, &name, &pane) {
                Ok(()) => return Ok(name),
                Err(e) if e.kind == ErrorKind::PaneBusy => continue,
                Err(e) => return Err(e),
            }
        }
        let from = st
            .last_pane
            .clone()
            .or_else(|| st.root_pane.clone())
            .unwrap_or_default();
        let dir = if st.splits.is_multiple_of(2) {
            Direction::Right
        } else {
            Direction::Down
        };
        let pane = self
            .herdr
            .pane_split(&from, dir, st.worktree_path.as_deref())?;
        st.splits += 1;
        st.last_pane = Some(pane.pane_id.clone());
        self.start_agent_in(st, stage, &name, &pane.pane_id)?;
        Ok(name)
    }

    fn start_agent_in(
        &self,
        st: &mut RunState,
        stage: &Stage,
        name: &str,
        pane: &str,
    ) -> Result<(), HerdrError> {
        let info =
            self.herdr
                .agent_start(name, &stage.kind, pane, &stage.args, AGENT_START_TIMEOUT_MS)?;
        st.agents.insert(
            stage.role.clone(),
            AgentSlot {
                name: name.to_string(),
                kind: stage.kind.clone(),
                pane_id: info.pane_id.clone(),
            },
        );
        if st.last_pane.is_none() {
            st.last_pane = Some(info.pane_id.clone());
        }
        st.event(
            "agent_start",
            Some(&stage.id),
            format!("started {name} ({}) in {}", stage.kind, info.pane_id),
            None,
        );
        Ok(())
    }

    /// Verify the stage's artifact and the read-only rule.
    fn check_artifact(&self, st: &RunState, stage: &Stage) -> Result<StageResult> {
        let path = self.artifact_path(st, stage);
        let wt = st.worktree_path.clone().context("no worktree")?;
        let rel = rel_to(&path, &wt);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => {
                return Ok(StageResult::Escalate(format!(
                    "{MISSING}: {rel} (the agent settled without writing it)"
                )))
            }
        };
        if text.trim().is_empty() {
            return Ok(StageResult::Escalate(format!("{MISSING}: {rel} is empty")));
        }
        if let Some(question) = blocked_section(&text) {
            return Ok(StageResult::Escalate(format!(
                "{BLOCKED} in {rel}: {question}"
            )));
        }
        if stage.read_only {
            if let Some(since) = &st.cursor.stage_start_head {
                let mut outside = self.git.changed_outside(&wt, since, &run_dir_rel(&st.id))?;
                outside.retain(|f| !st.cursor.stage_start_dirty.contains(f));
                if !outside.is_empty() {
                    return Ok(StageResult::Escalate(format!(
                        "read-only stage {} changed files outside the run dir: {}",
                        stage.id,
                        outside.join(", ")
                    )));
                }
            }
        }
        if stage.verdict {
            let first = text.lines().next().unwrap_or("").trim();
            return Ok(match first {
                "VERDICT: APPROVE" => StageResult::Complete,
                "VERDICT: CHANGES" => StageResult::Changes,
                other => StageResult::Escalate(format!(
                    "line 1 of {rel} must be `VERDICT: APPROVE` or `VERDICT: CHANGES`, got {other:?}"
                )),
            });
        }
        Ok(StageResult::Complete)
    }

    fn escalate(&self, st: &mut RunState, stage: &str, reason: String) -> Result<Outcome> {
        st.status = RunStatus::NeedsHuman;
        st.reason = Some(reason.clone());
        st.event("escalated", Some(stage), &reason, None);
        self.save(st)?;
        if let Err(e) = self.herdr.notify(
            &format!("{}: {stage} needs you", st.id),
            &truncate(&reason, 200),
            Sound::Request,
        ) {
            st.event("warning", Some(stage), format!("notify: {e}"), None);
            self.save(st)?;
        }
        self.say(&st.id, format!("{stage} needs a human: {reason}"));
        self.say(
            &st.id,
            format!(
                "fix it (finish {} by hand, or leave notes in {}/{HUMAN_NOTES}), then run: muster resume {}",
                stage,
                st.run_dir()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default(),
                st.id
            ),
        );
        Ok(Outcome::from_state(st))
    }

    // --------------------------------------------------------------- finish

    /// Merge `<remote>/<base_branch>` into the run branch if it has moved.
    /// Returns `Some(reason)` when the run must escalate.
    fn merge_base(&self, st: &mut RunState, wt: &Path) -> Result<Option<String>> {
        let base = format!("{}/{}", st.remote, st.base_branch);
        if let Err(e) = self.git.fetch_branch(wt, &st.remote, &st.base_branch) {
            let reason = format!("finish: git fetch {base} failed: {e:#}");
            st.event("error", None, &reason, None);
            return Ok(Some(reason));
        }
        match self.git.is_ancestor(wt, &base, "HEAD") {
            Ok(true) => return Ok(None),
            Ok(false) => {}
            Err(e) => {
                let reason = format!("finish: merge {base} failed: {e:#}");
                st.event("error", None, &reason, None);
                return Ok(Some(reason));
            }
        }
        let msg = format!("muster: merge {base} into {} for {}", st.branch, st.id);
        match self.git.merge(wt, &base, &msg) {
            Ok(MergeOutcome::Merged) => {
                let sha = self.git.head(wt).unwrap_or_default();
                let short = &sha[..sha.len().min(7)];
                st.event(
                    "merged",
                    None,
                    format!("{base} into {} ({short})", st.branch),
                    None,
                );
                self.save(st)?;
                Ok(None)
            }
            Ok(MergeOutcome::Conflict(files)) => {
                let reason = format!(
                    "finish: merging {base} into {} conflicts in {}; merge it by hand in {} (git merge {base}, resolve, commit), then run: muster resume {}",
                    st.branch,
                    files.join(", "),
                    wt.display(),
                    st.id
                );
                st.event("error", None, &reason, None);
                Ok(Some(reason))
            }
            Err(e) => {
                let reason = format!("finish: merge {base} failed: {e:#}");
                st.event("error", None, &reason, None);
                Ok(Some(reason))
            }
        }
    }

    fn finish(&self, st: &mut RunState, loaded: &Loaded) -> Result<Outcome> {
        let wt = st.worktree_path.clone().context("no worktree")?;
        let run_dir = st.run_dir().context("no run dir")?;
        let last_stage = st
            .events
            .iter()
            .rev()
            .find(|e| e.kind == "stage_done")
            .and_then(|e| e.stage.clone())
            .unwrap_or_else(|| "the last stage".into());

        let step = |st: &mut RunState, r: Result<()>, what: &str| -> Option<String> {
            r.err().map(|e| {
                let reason = format!("finish: {what} failed: {e:#}");
                st.event("error", None, &reason, None);
                reason
            })
        };

        if self.git.merge_in_progress(&wt) {
            let reason = format!(
                "finish: a merge is in progress in {}; finish it (resolve and commit) or run git merge --abort, then run: muster resume {}",
                wt.display(),
                st.id
            );
            st.event("error", None, &reason, None);
            return self.escalate(st, "finish", reason);
        }

        if self.git.is_dirty(&wt)? {
            let r = self.git.commit_all(
                &wt,
                &format!("muster: uncommitted changes after {last_stage}"),
            );
            if let Some(reason) = step(st, r.map(|_| ()), "leftover commit") {
                return self.escalate(st, "finish", reason);
            }
            st.event(
                "warning",
                None,
                format!("agents left uncommitted changes after {last_stage}; committed them"),
                None,
            );
        }

        if self.cfg.entire.enabled && which(&self.git.entire_bin).is_some() {
            match self.git.entire_outgoing(&wt) {
                Ok(out) => {
                    let detail = serde_json::from_str::<Value>(&out).unwrap_or(Value::String(out));
                    st.event("entire", None, "outgoing checkpoints", Some(detail));
                }
                Err(e) => st.event(
                    "warning",
                    None,
                    format!("entire checkpoint list: {e:#}"),
                    None,
                ),
            }
        }

        let finish = &loaded.workflow.finish;
        if finish.open_pr {
            if let Some(reason) = self.merge_base(st, &wt)? {
                return self.escalate(st, "finish", reason);
            }
            let r = self.git.push(&wt, &st.remote, &st.branch);
            if let Some(reason) = step(st, r, "git push") {
                return self.escalate(st, "finish", reason);
            }
            st.event("pushed", None, format!("{} {}", st.remote, st.branch), None);
            self.save(st)?;

            if st.pr_url.is_none() {
                let brief = std::fs::read_to_string(run_dir.join("brief.md")).unwrap_or_default();
                let title = pr_title(&brief, &st.id);
                let body = pr_body(&run_dir, &finish.pr_body_from, st);
                let body_path = self.store.run_data_dir(&st.id).join("pr-body.md");
                std::fs::create_dir_all(body_path.parent().unwrap())?;
                std::fs::write(&body_path, body)?;
                match self.git.pr_create(
                    &wt,
                    &st.base_branch,
                    &st.branch,
                    &title,
                    &body_path,
                    finish.draft,
                ) {
                    Ok(url) => {
                        st.pr_url = Some(url.clone());
                        st.event("pr", None, url, None);
                    }
                    Err(e) => {
                        let reason = format!("finish: gh pr create failed: {e:#}");
                        st.event("error", None, &reason, None);
                        return self.escalate(st, "finish", reason);
                    }
                }
            }
        }

        st.status = RunStatus::Done;
        st.reason = None;
        st.event("done", None, "run finished", None);
        self.save(st)?;
        let body = st
            .pr_url
            .clone()
            .unwrap_or_else(|| format!("artifacts in {}", run_dir.display()));
        if let Err(e) = self
            .herdr
            .notify(&format!("{} ready", st.id), &body, Sound::Done)
        {
            st.event("warning", None, format!("notify: {e}"), None);
            self.save(st)?;
        }
        self.say(&st.id, format!("done: {body}"));
        Ok(Outcome::from_state(st))
    }

    // ---------------------------------------------------------------- clean

    pub fn clean(&self, id: &str, delete_branch: bool, force: bool) -> Result<Value> {
        let mut st = self.store.load(id)?;
        let lock = match self.store.lock(id) {
            Ok(l) => Some(l),
            Err(_) if force => None,
            Err(_) => bail!(MusterError::usage(format!(
                "run {id} is running in another process; use --force to clean it anyway"
            ))),
        };
        if st.status == RunStatus::Running && lock.is_none() && !force {
            bail!(MusterError::usage(format!(
                "run {id} is running; use --force"
            )));
        }
        let mut notes = Vec::new();
        for slot in st.agents.values() {
            match self.herdr.pane_close(&slot.pane_id) {
                Ok(()) => notes.push(format!("closed pane {} ({})", slot.pane_id, slot.name)),
                Err(e) if e.is_not_found() => {}
                Err(e) => notes.push(format!("pane {}: {e}", slot.pane_id)),
            }
        }
        if let Some(ws) = &st.workspace_id {
            match self.herdr.worktree_remove(ws, force) {
                Ok(()) => notes.push(format!("removed worktree (workspace {ws})")),
                Err(e) if e.is_not_found() => notes.push(format!("workspace {ws} already gone")),
                Err(e) => bail!(MusterError::failed(format!(
                    "worktree remove failed: {e}{}",
                    if force {
                        ""
                    } else {
                        " (use --force to discard uncommitted changes)"
                    }
                ))),
            }
        }
        if delete_branch && self.git.branch_exists(&st.repo_path, &st.branch) {
            self.git.delete_branch(&st.repo_path, &st.branch)?;
            notes.push(format!("deleted branch {}", st.branch));
        }
        if st.status == RunStatus::Running {
            st.status = RunStatus::Failed;
            st.reason = Some("cleaned while running".into());
        }
        st.cleaned_at = Some(now());
        st.event("cleaned", None, notes.join("; "), None);
        self.save(&mut st)?;
        Ok(json!({"ok": true, "run_id": id, "actions": notes}))
    }
}

const MISSING: &str = "artifact missing";
const BLOCKED: &str = "agent wrote ## Blocked";

/// Escalations that resume handles by prompting the stage again.
fn is_redo(reason: &str) -> bool {
    reason.starts_with(MISSING) || reason.starts_with(BLOCKED)
}

/// The text under a `## Blocked` heading, if present.
pub fn blocked_section(text: &str) -> Option<String> {
    let mut lines = text.lines();
    lines.find(|l| {
        let t = l.trim();
        t.eq_ignore_ascii_case("## blocked") || t.to_ascii_lowercase().starts_with("## blocked ")
    })?;
    let body: Vec<&str> = lines
        .take_while(|l| !l.starts_with("## "))
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    Some(if body.is_empty() {
        "(no question given)".into()
    } else {
        truncate(&body.join(" "), 300)
    })
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

/// Move `path` to the first free `<stem>-<k>.<ext>` and return the new path.
fn archive(path: &Path) -> Result<PathBuf> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("artifact");
    let ext = path.extension().and_then(|s| s.to_str());
    for k in 1.. {
        let name = match ext {
            Some(e) => format!("{stem}-{k}.{e}"),
            None => format!("{stem}-{k}"),
        };
        let dest = path.with_file_name(name);
        if !dest.exists() {
            std::fs::rename(path, &dest)?;
            return Ok(dest);
        }
    }
    unreachable!()
}

fn rel_to(p: &Path, base: &Path) -> String {
    p.strip_prefix(base).unwrap_or(p).display().to_string()
}

fn describe_herdr(e: &HerdrError, what: &str) -> String {
    let hint = match e.kind {
        ErrorKind::Timeout => {
            " (the prompt may still have been delivered; check the agent before resuming)"
        }
        ErrorKind::PromptStalled => " (no activity after the prompt was sent)",
        ErrorKind::NotReady => " (the agent may be showing a trust or onboarding prompt)",
        ErrorKind::Blocked => " (the agent is waiting on a dialog)",
        _ => "",
    };
    format!("{what} failed: {}: {}{hint}", e.code, e.message)
}

fn raw(e: &HerdrError) -> Value {
    json!({"code": e.code, "command": e.command, "exit_code": e.raw.exit_code, "stdout": e.raw.stdout, "stderr": e.raw.stderr})
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &dest)?;
        } else {
            std::fs::copy(e.path(), dest)?;
        }
    }
    Ok(())
}

/// First non-empty line of the brief, without Markdown heading marks.
pub fn pr_title(brief: &str, id: &str) -> String {
    let line = brief
        .lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty())
        .unwrap_or(id);
    truncate(line, 100)
}

pub fn pr_body(run_dir: &Path, files: &[String], st: &RunState) -> String {
    let mut body = String::new();
    for f in files {
        if let Ok(text) = std::fs::read_to_string(run_dir.join(f)) {
            body.push_str(&format!("## {f}\n\n{}\n\n", text.trim_end()));
        }
    }
    body.push_str(&format!(
        "---\nmuster run `{}` · workflow `{}` · run dir `{}/`\n",
        st.id,
        st.workflow,
        run_dir_rel(&st.id).display()
    ));
    body
}

// ---------------------------------------------------------------- status

#[derive(Debug, Serialize)]
pub struct StatusRow {
    pub id: String,
    pub workflow: String,
    pub repo: String,
    pub stage: String,
    pub iteration: u32,
    pub status: String,
    pub age: String,
    pub stale: bool,
}

pub fn status_rows(store: &StateStore) -> Result<Vec<StatusRow>> {
    Ok(store
        .list()?
        .into_iter()
        .map(|s| {
            let stale = s.status == RunStatus::Running && !store.is_locked(&s.id);
            StatusRow {
                stage: s.cursor.stage.clone().unwrap_or_else(|| "finish".into()),
                iteration: s.cursor.iteration,
                status: s.status.as_str().into(),
                age: age(&s.created_at),
                stale,
                workflow: s.workflow,
                repo: s.repo,
                id: s.id,
            }
        })
        .collect())
}

/// Detail for one run, including whether the live config changed.
pub fn status_detail(store: &StateStore, cfg_dir: &Path, id: &str) -> Result<Value> {
    let st = store.load(id)?;
    let roles: BTreeSet<String> = workflow::load(&store.snapshot_dir(id), &st.workflow)
        .map(|l| l.workflow.roles().into_iter().map(str::to_string).collect())
        .unwrap_or_default();
    let live = state::live_hash(cfg_dir, &st.workflow, &roles);
    let config_changed = live.as_deref().map(|h| h != st.config_hash);
    let stale = st.status == RunStatus::Running && !store.is_locked(id);
    let mut v = serde_json::to_value(&st)?;
    v["live_config_changed"] = json!(config_changed);
    v["stale"] = json!(stale);
    v["age"] = json!(age(&st.created_at));
    v["run_dir"] = json!(st.run_dir());
    Ok(v)
}

fn age(created: &str) -> String {
    use time::format_description::well_known::Rfc3339;
    let Ok(t) = time::OffsetDateTime::parse(created, &Rfc3339) else {
        return "?".into();
    };
    let secs = (time::OffsetDateTime::now_utc() - t).whole_seconds().max(0);
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_detection() {
        assert_eq!(blocked_section("# Plan\nok"), None);
        assert_eq!(
            blocked_section("x\n## Blocked\n\nWhich DB?\nPostgres or SQLite?\n## Other\nz")
                .unwrap(),
            "Which DB? Postgres or SQLite?"
        );
        assert_eq!(
            blocked_section("## BLOCKED").unwrap(),
            "(no question given)"
        );
        assert!(blocked_section("### Blocked\n").is_none());
        assert!(blocked_section("## Blockers resolved\n").is_none());
    }

    #[test]
    fn archive_picks_free_name() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("review.md");
        std::fs::write(&p, "1").unwrap();
        assert_eq!(archive(&p).unwrap(), tmp.path().join("review-1.md"));
        std::fs::write(&p, "2").unwrap();
        assert_eq!(archive(&p).unwrap(), tmp.path().join("review-2.md"));
        assert!(!p.exists());
    }

    #[test]
    fn titles() {
        assert_eq!(pr_title("\n# Add retries\n\nbody", "f1"), "Add retries");
        assert_eq!(pr_title("   \n", "f1"), "f1");
        assert_eq!(pr_title(&"x".repeat(150), "f1").chars().count(), 101);
    }
}
