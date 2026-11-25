//! Run state persistence, per-run locks and config snapshots.
//!
//! Layout under `state_dir`:
//! - `<run>.json`: run state (written atomically: temp file, then rename)
//! - `<run>.lock`: held (flock) by the process driving the run
//! - `<run>/config/`: the config snapshot the run reads from
//! - `<run>/pr-body.md`: the assembled PR body
//! - `.start.lock`: serializes the concurrency check and run creation

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::config::{PROMPTS_DIR, WORKFLOWS_DIR};
use crate::error::MusterError;
use crate::workflow::{Loaded, MAX_RUN_ID_LEN};

pub const STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    NeedsHuman,
    Done,
    Failed,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::NeedsHuman => "needs_human",
            RunStatus::Done => "done",
            RunStatus::Failed => "failed",
        }
    }
}

/// The agent playing a role. The agent name is the primary handle; the pane
/// id is only used for splitting and metadata, since pane ids can change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSlot {
    pub name: String,
    pub kind: String,
    pub pane_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub at: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

/// Where the run is. `stage == None` means the stages are finished and the
/// finish step is next (or done).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub stage: Option<String>,
    pub iteration: u32,
    /// The prompt for the current stage may have been delivered. On resume,
    /// the artifact is checked before prompting again.
    pub prompted: bool,
    /// HEAD when the current stage was prompted (for read-only checks).
    #[serde(default)]
    pub stage_start_head: Option<String>,
    /// Paths already dirty or untracked when the stage was prompted. A
    /// read-only stage isn't blamed for them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stage_start_dirty: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunState {
    pub version: u32,
    pub id: String,
    pub workflow: String,
    pub repo: String,
    pub repo_path: PathBuf,
    pub branch: String,
    pub base_branch: String,
    pub remote: String,
    #[serde(default)]
    pub worktree_path: Option<PathBuf>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub root_pane: Option<String>,
    /// role -> agent
    #[serde(default)]
    pub agents: BTreeMap<String, AgentSlot>,
    /// Most recently created pane, the source for the next split.
    #[serde(default)]
    pub last_pane: Option<String>,
    #[serde(default)]
    pub splits: u32,
    #[serde(default)]
    pub skip: BTreeSet<String>,
    pub cursor: Cursor,
    #[serde(default)]
    pub previous_review: Option<PathBuf>,
    pub status: RunStatus,
    #[serde(default)]
    pub reason: Option<String>,
    pub config_hash: String,
    #[serde(default)]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub cleaned_at: Option<String>,
    #[serde(default)]
    pub events: Vec<Event>,
}

pub fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

impl RunState {
    pub fn run_dir(&self) -> Option<PathBuf> {
        self.worktree_path
            .as_ref()
            .map(|w| w.join(run_dir_rel(&self.id)))
    }

    pub fn event(
        &mut self,
        kind: &str,
        stage: Option<&str>,
        message: impl Into<String>,
        detail: Option<Value>,
    ) {
        let iteration = stage.map(|_| self.cursor.iteration);
        self.events.push(Event {
            at: now(),
            kind: kind.to_string(),
            stage: stage.map(str::to_string),
            iteration,
            message: message.into(),
            detail,
        });
    }
}

/// The run dir, relative to the worktree root.
pub fn run_dir_rel(run_id: &str) -> PathBuf {
    Path::new(".muster").join("runs").join(run_id)
}

/// Run ids are `[a-z][a-z0-9]*`, at most 11 characters, so `<role>-<run>`
/// stays a valid herdr agent name.
pub fn check_run_id(id: &str) -> Result<()> {
    let mut c = id.chars();
    let ok = id.len() <= MAX_RUN_ID_LEN
        && matches!(c.next(), Some(x) if x.is_ascii_lowercase())
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit());
    if !ok {
        bail!(MusterError::usage(format!(
            "invalid run id {id:?}: use [a-z][a-z0-9]*, at most {MAX_RUN_ID_LEN} characters"
        )));
    }
    Ok(())
}

/// Held for as long as a process drives a run. Released on drop or exit.
#[derive(Debug)]
pub struct RunLock {
    _file: File,
}

#[derive(Debug, Clone)]
pub struct StateStore {
    pub dir: PathBuf,
}

impl StateStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn ensure(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating state dir {}", self.dir.display()))
    }

    pub fn state_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn lock_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.lock"))
    }

    pub fn run_data_dir(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    pub fn snapshot_dir(&self, id: &str) -> PathBuf {
        self.run_data_dir(id).join("config")
    }

    pub fn exists(&self, id: &str) -> bool {
        self.state_path(id).exists()
    }

    pub fn load(&self, id: &str) -> Result<RunState> {
        check_run_id(id)?;
        let p = self.state_path(id);
        let text = std::fs::read_to_string(&p).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::Error::new(MusterError::usage(format!("no run named {id}")))
            } else {
                anyhow::Error::new(e).context(format!("reading {}", p.display()))
            }
        })?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", p.display()))
    }

    /// Write atomically: temp file in the same dir, fsync, rename.
    pub fn save(&self, state: &mut RunState) -> Result<()> {
        use std::io::Write;
        state.updated_at = now();
        let p = self.state_path(&state.id);
        let tmp = self.dir.join(format!(".{}.json.tmp", state.id));
        {
            let mut f = File::create(&tmp)?;
            f.write_all(serde_json::to_string_pretty(state)?.as_bytes())?;
            f.write_all(b"\n")?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &p).with_context(|| format!("writing {}", p.display()))
    }

    /// All runs, newest first.
    pub fn list(&self) -> Result<Vec<RunState>> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Ok(out);
        };
        for e in entries {
            let p = e?.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || !name.ends_with(".json") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Ok(s) = serde_json::from_str::<RunState>(&text) {
                    out.push(s);
                }
            }
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        Ok(out)
    }

    fn try_lock_file(path: &Path) -> Result<Option<File>> {
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        match f.try_lock() {
            Ok(()) => Ok(Some(f)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => Err(e.into()),
        }
    }

    /// Take the run's lock, or fail if another process holds it.
    pub fn lock(&self, id: &str) -> Result<RunLock> {
        match Self::try_lock_file(&self.lock_path(id))? {
            Some(f) => Ok(RunLock { _file: f }),
            None => bail!(MusterError::usage(format!(
                "run {id} is being driven by another muster process"
            ))),
        }
    }

    /// Whether some process holds the run's lock.
    pub fn is_locked(&self, id: &str) -> bool {
        if !self.lock_path(id).exists() {
            return false;
        }
        matches!(Self::try_lock_file(&self.lock_path(id)), Ok(None))
    }

    /// Serialize run creation across processes (blocking).
    pub fn start_lock(&self) -> Result<RunLock> {
        self.ensure()?;
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join(".start.lock"))?;
        f.lock()?;
        Ok(RunLock { _file: f })
    }

    /// Runs that count against `max_concurrent_runs`: status `running` and
    /// actually held by a live process. A `running` run whose lock is free
    /// was interrupted (crash, kill) and does not count.
    pub fn active(&self, except: Option<&str>) -> Result<Vec<String>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|s| s.status == RunStatus::Running && Some(s.id.as_str()) != except)
            .filter(|s| self.is_locked(&s.id))
            .map(|s| s.id)
            .collect())
    }

    /// Next id for a workflow: first letter plus one more than the highest
    /// existing number with that letter, for example `f12`.
    pub fn next_run_id(&self, workflow: &str) -> Result<String> {
        let letter = workflow
            .chars()
            .next()
            .filter(char::is_ascii_lowercase)
            .unwrap_or('r');
        let mut max = 0u64;
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for e in entries.flatten() {
                let name = e.file_name();
                let name = name.to_string_lossy();
                let Some(stem) = name.strip_suffix(".json") else {
                    continue;
                };
                if let Some(n) = stem
                    .strip_prefix(letter)
                    .and_then(|r| r.parse::<u64>().ok())
                {
                    max = max.max(n);
                }
            }
        }
        Ok(format!("{letter}{}", max + 1))
    }
}

/// Files of a workflow snapshot: relative path -> contents.
pub fn snapshot_files(loaded: &Loaded) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    files.insert(
        format!("{WORKFLOWS_DIR}/{}.toml", loaded.name),
        loaded.source.clone(),
    );
    for (role, text) in &loaded.prompts {
        files.insert(format!("{PROMPTS_DIR}/{role}.md"), text.clone());
    }
    files
}

/// SHA-256 over sorted (path, contents) pairs.
pub fn hash_files(files: &BTreeMap<String, String>) -> String {
    let mut h = Sha256::new();
    for (path, text) in files {
        h.update(path.as_bytes());
        h.update([0]);
        h.update((text.len() as u64).to_le_bytes());
        h.update(text.as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

pub fn write_files(dir: &Path, files: &BTreeMap<String, String>) -> Result<()> {
    for (rel, text) in files {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, text).with_context(|| format!("writing {}", p.display()))?;
    }
    Ok(())
}

/// Hash of what the live config dir currently has for this run's workflow,
/// without validating it. `None` if it can no longer be read.
pub fn live_hash(config_dir: &Path, workflow: &str, roles: &BTreeSet<String>) -> Option<String> {
    let mut files = BTreeMap::new();
    let wf = format!("{WORKFLOWS_DIR}/{workflow}.toml");
    files.insert(
        wf.clone(),
        std::fs::read_to_string(config_dir.join(&wf)).ok()?,
    );
    for role in roles {
        let p = format!("{PROMPTS_DIR}/{role}.md");
        files.insert(
            p.clone(),
            std::fs::read_to_string(config_dir.join(&p)).ok()?,
        );
    }
    Some(hash_files(&files))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, status: RunStatus) -> RunState {
        RunState {
            version: STATE_VERSION,
            id: id.into(),
            workflow: "feature".into(),
            repo: "r".into(),
            repo_path: "/r".into(),
            branch: format!("muster/feat/{id}"),
            base_branch: "main".into(),
            remote: "origin".into(),
            worktree_path: None,
            workspace_id: None,
            root_pane: None,
            agents: BTreeMap::new(),
            last_pane: None,
            splits: 0,
            skip: BTreeSet::new(),
            cursor: Cursor {
                stage: Some("plan".into()),
                iteration: 1,
                prompted: false,
                stage_start_head: None,
                stage_start_dirty: Vec::new(),
            },
            previous_review: None,
            status,
            reason: None,
            config_hash: "h".into(),
            pr_url: None,
            pid: None,
            created_at: now(),
            updated_at: now(),
            cleaned_at: None,
            events: vec![],
        }
    }

    #[test]
    fn save_load_roundtrip_and_list() {
        let tmp = tempfile::tempdir().unwrap();
        let store = StateStore::new(tmp.path());
        let mut s = sample("f1", RunStatus::Running);
        s.event("start", Some("plan"), "hello", None);
        store.save(&mut s).unwrap();
        let back = store.load("f1").unwrap();
        assert_eq!(back.events.len(), 1);
        assert_eq!(back.events[0].iteration, Some(1));
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(!tmp.path().join(".f1.json.tmp").exists());
        assert!(store.load("f2").is_err());
    }

    #[test]
    fn run_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let store = StateStore::new(tmp.path());
        assert_eq!(store.next_run_id("feature").unwrap(), "f1");
        for id in ["f1", "f9", "f12", "b3"] {
            store.save(&mut sample(id, RunStatus::Done)).unwrap();
        }
        assert_eq!(store.next_run_id("feature").unwrap(), "f13");
        assert_eq!(store.next_run_id("bugfix").unwrap(), "b4");
        assert_eq!(store.next_run_id("investigate").unwrap(), "i1");

        assert!(check_run_id("f12").is_ok());
        for bad in ["", "F1", "1f", "f-1", "abcdefghijkl"] {
            assert!(check_run_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn locks_and_active_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let store = StateStore::new(tmp.path());
        store.save(&mut sample("f1", RunStatus::Running)).unwrap();
        store.save(&mut sample("f2", RunStatus::Running)).unwrap();
        store
            .save(&mut sample("f3", RunStatus::NeedsHuman))
            .unwrap();

        // Nothing is locked: interrupted runs don't count.
        assert!(store.active(None).unwrap().is_empty());

        let lock = store.lock("f1").unwrap();
        assert!(store.is_locked("f1"));
        assert!(store.lock("f1").is_err());
        assert_eq!(store.active(None).unwrap(), vec!["f1".to_string()]);
        assert!(store.active(Some("f1")).unwrap().is_empty());
        drop(lock);
        assert!(!store.is_locked("f1"));
    }

    #[test]
    fn hash_changes_with_content() {
        let mut a = BTreeMap::new();
        a.insert("workflows/x.toml".to_string(), "a".to_string());
        let h1 = hash_files(&a);
        a.insert("prompts/p.md".to_string(), "b".to_string());
        let h2 = hash_files(&a);
        assert_ne!(h1, h2);
        assert_eq!(h2, hash_files(&a.clone()));
        assert_eq!(h1.len(), 64);
    }
}
