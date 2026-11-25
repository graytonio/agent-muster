//! Every herdr CLI call lives here, behind the [`Herdr`] trait.
//!
//! Flags and response shapes were checked against herdr 0.9.3 (`herdr <group>`
//! usage text, `herdr api schema --json`, and recorded responses in
//! `tests/fixtures/herdr/`). Successful calls print `{"id":..,"result":{..}}`
//! on stdout. Server errors print `{"error":{"code":..,"message":..}}` on
//! stderr and exit 1. CLI syntax errors print text and exit 2.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The caller's `--timeout` expired.
    Timeout,
    /// `agent prompt --wait` saw no activity after submission.
    PromptStalled,
    /// The agent started but is not ready (for example a trust prompt).
    NotReady,
    /// The agent is waiting on an approval or question UI.
    Blocked,
    /// The agent, pane or workspace does not exist.
    NotFound,
    /// The pane is not an idle shell.
    PaneBusy,
    /// herdr rejected the command line (exit 2).
    Usage,
    /// The herdr binary could not be run.
    Spawn,
    /// The output could not be parsed.
    Parse,
    Other,
}

#[derive(Debug, Clone, Error, Serialize)]
#[error("herdr {command}: {code}: {message}")]
pub struct HerdrError {
    pub kind: ErrorKind,
    /// herdr's error code, for example `agent_prompt_stalled`.
    pub code: String,
    pub message: String,
    /// The subcommand, for example `agent prompt`.
    pub command: String,
    /// Exit code and raw output, preserved for the event log.
    pub raw: Box<RawOutput>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RawOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl HerdrError {
    pub fn is_not_found(&self) -> bool {
        self.kind == ErrorKind::NotFound
    }
}

pub type HResult<T> = Result<T, HerdrError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Right,
    Down,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Right => "right",
            Direction::Down => "down",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    None,
    Done,
    Request,
}

impl Sound {
    pub fn as_str(self) -> &'static str {
        match self {
            Sound::None => "none",
            Sound::Done => "done",
            Sound::Request => "request",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneInfo {
    pub pane_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub tab_id: String,
    #[serde(default)]
    pub agent_status: String,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    pub pane_id: String,
    pub workspace_id: String,
    /// `idle | working | blocked | done | unknown`
    pub agent_status: String,
}

impl AgentInfo {
    /// `idle` and `done` both mean ready for input.
    pub fn is_settled(&self) -> bool {
        matches!(self.agent_status.as_str(), "idle" | "done")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeCreated {
    pub workspace_id: String,
    pub root_pane: String,
    pub path: PathBuf,
    pub branch: String,
}

#[derive(Debug, Clone)]
pub struct WorktreeRequest<'a> {
    pub repo_path: &'a Path,
    pub branch: &'a str,
    pub base: Option<&'a str>,
    pub path: Option<&'a Path>,
    pub label: &'a str,
}

/// The herdr operations muster needs. Each method is one CLI call.
pub trait Herdr {
    fn status(&self) -> HResult<Value>;
    fn worktree_create(&self, req: &WorktreeRequest) -> HResult<WorktreeCreated>;
    fn worktree_remove(&self, workspace_id: &str, force: bool) -> HResult<()>;
    fn pane_split(&self, pane: &str, direction: Direction, cwd: Option<&Path>)
        -> HResult<PaneInfo>;
    fn pane_get(&self, pane: &str) -> HResult<PaneInfo>;
    fn pane_close(&self, pane: &str) -> HResult<()>;
    fn agent_start(
        &self,
        name: &str,
        kind: &str,
        pane: &str,
        args: &[String],
        timeout_ms: u64,
    ) -> HResult<AgentInfo>;
    /// Submit `text` and wait for the first settled state (`idle`, `done` or
    /// `blocked`). This does not track the individual turn.
    fn agent_prompt_wait(&self, name: &str, text: &str, timeout_ms: u64) -> HResult<AgentInfo>;
    /// `Ok(None)` when no live agent has this name.
    fn agent_get(&self, name: &str) -> HResult<Option<AgentInfo>>;
    fn agent_read(&self, name: &str, lines: u32) -> HResult<String>;
    fn agent_send_keys(&self, name: &str, keys: &[&str]) -> HResult<()>;
    fn notify(&self, title: &str, body: &str, sound: Sound) -> HResult<()>;
    fn report_metadata(&self, pane: &str, tokens: &[(&str, String)]) -> HResult<()>;
    /// Raw `herdr integration status` text.
    fn integration_status(&self) -> HResult<String>;
}

/// Builds argument vectors. Kept separate from execution so tests can check
/// the exact flags.
#[derive(Debug, Clone)]
pub struct Args {
    pub session: Option<String>,
}

impl Args {
    fn base(&self) -> Vec<String> {
        match &self.session {
            Some(s) => vec!["--session".into(), s.clone()],
            None => Vec::new(),
        }
    }

    fn with(&self, rest: &[&str]) -> Vec<String> {
        let mut v = self.base();
        v.extend(rest.iter().map(|s| s.to_string()));
        v
    }

    pub fn status(&self) -> Vec<String> {
        self.with(&["status", "server", "--json"])
    }

    pub fn worktree_create(&self, r: &WorktreeRequest) -> Vec<String> {
        let mut v = self.with(&["worktree", "create", "--cwd"]);
        v.push(r.repo_path.display().to_string());
        v.extend(["--branch".into(), r.branch.to_string()]);
        if let Some(b) = r.base {
            v.extend(["--base".into(), b.to_string()]);
        }
        if let Some(p) = r.path {
            v.extend(["--path".into(), p.display().to_string()]);
        }
        v.extend(["--label".into(), r.label.to_string(), "--no-focus".into()]);
        v
    }

    pub fn worktree_remove(&self, workspace_id: &str, force: bool) -> Vec<String> {
        let mut v = self.with(&["worktree", "remove", "--workspace", workspace_id]);
        if force {
            v.push("--force".into());
        }
        v
    }

    pub fn pane_split(&self, pane: &str, d: Direction, cwd: Option<&Path>) -> Vec<String> {
        let mut v = self.with(&["pane", "split", pane, "--direction", d.as_str()]);
        if let Some(c) = cwd {
            v.extend(["--cwd".into(), c.display().to_string()]);
        }
        v.push("--no-focus".into());
        v
    }

    pub fn pane_get(&self, pane: &str) -> Vec<String> {
        self.with(&["pane", "get", pane])
    }

    pub fn pane_close(&self, pane: &str) -> Vec<String> {
        self.with(&["pane", "close", pane])
    }

    pub fn agent_start(
        &self,
        name: &str,
        kind: &str,
        pane: &str,
        args: &[String],
        timeout_ms: u64,
    ) -> Vec<String> {
        let t = timeout_ms.to_string();
        let mut v = self.with(&[
            "agent",
            "start",
            name,
            "--kind",
            kind,
            "--pane",
            pane,
            "--timeout",
            &t,
        ]);
        if !args.is_empty() {
            v.push("--".into());
            v.extend(args.iter().cloned());
        }
        v
    }

    pub fn agent_prompt_wait(&self, name: &str, text: &str, timeout_ms: u64) -> Vec<String> {
        // `--wait` alone waits for the first settled idle/done/blocked state;
        // herdr's docs say not to repeat those defaults with `--until`.
        let t = timeout_ms.to_string();
        self.with(&["agent", "prompt", name, text, "--wait", "--timeout", &t])
    }

    pub fn agent_get(&self, name: &str) -> Vec<String> {
        self.with(&["agent", "get", name])
    }

    pub fn agent_read(&self, name: &str, lines: u32) -> Vec<String> {
        let l = lines.to_string();
        self.with(&[
            "agent",
            "read",
            name,
            "--source",
            "recent-unwrapped",
            "--lines",
            &l,
        ])
    }

    pub fn agent_send_keys(&self, name: &str, keys: &[&str]) -> Vec<String> {
        let mut v = self.with(&["agent", "send-keys", name]);
        v.extend(keys.iter().map(|k| k.to_string()));
        v
    }

    pub fn notify(&self, title: &str, body: &str, sound: Sound) -> Vec<String> {
        self.with(&[
            "notification",
            "show",
            title,
            "--body",
            body,
            "--sound",
            sound.as_str(),
        ])
    }

    pub fn report_metadata(&self, pane: &str, tokens: &[(&str, String)]) -> Vec<String> {
        let mut v = self.with(&["pane", "report-metadata", pane, "--source", "muster"]);
        for (k, val) in tokens {
            v.push("--token".into());
            v.push(format!("{k}={val}"));
        }
        v
    }

    pub fn integration_status(&self) -> Vec<String> {
        self.with(&["integration", "status"])
    }
}

/// Map a failed herdr invocation to a typed error.
pub fn map_error(command: &str, exit_code: Option<i32>, stdout: &str, stderr: &str) -> HerdrError {
    let parsed = [stderr, stdout]
        .iter()
        .find_map(|s| serde_json::from_str::<Value>(s.trim()).ok())
        .and_then(|v| v.get("error").cloned());
    let (code, message) = match &parsed {
        Some(e) => (
            e.get("code")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            e.get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ),
        None => (String::new(), first_line(stderr, stdout)),
    };
    let kind = if parsed.is_none() && exit_code == Some(2) {
        ErrorKind::Usage
    } else {
        kind_for_code(&code)
    };
    HerdrError {
        kind,
        code,
        message,
        command: command.to_string(),
        raw: Box::new(RawOutput {
            exit_code,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }),
    }
}

fn kind_for_code(code: &str) -> ErrorKind {
    match code {
        "timeout" | "agent_timeout" | "wait_timeout" => ErrorKind::Timeout,
        "agent_prompt_stalled" => ErrorKind::PromptStalled,
        "agent_not_ready" => ErrorKind::NotReady,
        "agent_blocked" => ErrorKind::Blocked,
        "agent_pane_busy" => ErrorKind::PaneBusy,
        c if c.ends_with("_not_found") => ErrorKind::NotFound,
        c if c.contains("timeout") => ErrorKind::Timeout,
        _ => ErrorKind::Other,
    }
}

fn first_line(a: &str, b: &str) -> String {
    a.lines()
        .chain(b.lines())
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("no output")
        .to_string()
}

/// Real implementation: shells out to the herdr binary.
pub struct CliHerdr {
    pub bin: String,
    pub args: Args,
    pub verbose: bool,
}

impl CliHerdr {
    pub fn new(bin: impl Into<String>, session: Option<String>, verbose: bool) -> Self {
        Self {
            bin: bin.into(),
            args: Args { session },
            verbose,
        }
    }

    /// Run herdr and return stdout, or a typed error.
    fn exec(&self, argv: Vec<String>) -> HResult<String> {
        let label = command_label(&argv);
        if self.verbose {
            eprintln!("+ {} {}", self.bin, shell_words(&argv));
        }
        let out = Command::new(&self.bin)
            .args(&argv)
            .output()
            .map_err(|e| HerdrError {
                kind: ErrorKind::Spawn,
                code: "spawn".into(),
                message: format!("cannot run {}: {e}", self.bin),
                command: label.clone(),
                raw: Box::new(RawOutput {
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                }),
            })?;
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if out.status.success() {
            Ok(stdout)
        } else {
            Err(map_error(&label, out.status.code(), &stdout, &stderr))
        }
    }

    fn exec_json(&self, argv: Vec<String>) -> HResult<Value> {
        let label = command_label(&argv);
        let stdout = self.exec(argv)?;
        serde_json::from_str(stdout.trim()).map_err(|e| parse_error(&label, &stdout, e))
    }
}

fn parse_error(label: &str, stdout: &str, e: impl std::fmt::Display) -> HerdrError {
    HerdrError {
        kind: ErrorKind::Parse,
        code: "parse".into(),
        message: format!("unexpected output: {e}"),
        command: label.to_string(),
        raw: Box::new(RawOutput {
            exit_code: Some(0),
            stdout: stdout.to_string(),
            stderr: String::new(),
        }),
    }
}

fn command_label(argv: &[String]) -> String {
    let skip = if argv.first().map(String::as_str) == Some("--session") {
        2
    } else {
        0
    };
    argv.iter()
        .skip(skip)
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_words(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if a.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./:=@".contains(c))
            {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Pull `result.<key>` out of a response and deserialize it.
fn result_field<T: serde::de::DeserializeOwned>(label: &str, v: &Value, key: &str) -> HResult<T> {
    let field = v
        .get("result")
        .and_then(|r| r.get(key))
        .cloned()
        .ok_or_else(|| parse_error(label, &v.to_string(), format!("missing result.{key}")))?;
    serde_json::from_value(field).map_err(|e| parse_error(label, &v.to_string(), e))
}

/// Parse `worktree create` output.
pub fn parse_worktree_created(v: &Value) -> HResult<WorktreeCreated> {
    let label = "worktree create";
    let pane: PaneInfo = result_field(label, v, "root_pane")?;
    let wt = v
        .get("result")
        .and_then(|r| r.get("worktree"))
        .ok_or_else(|| parse_error(label, &v.to_string(), "missing result.worktree"))?;
    let path = wt
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| parse_error(label, &v.to_string(), "missing result.worktree.path"))?;
    let branch = wt.get("branch").and_then(Value::as_str).unwrap_or_default();
    Ok(WorktreeCreated {
        workspace_id: pane.workspace_id.clone(),
        root_pane: pane.pane_id,
        path: PathBuf::from(path),
        branch: branch.to_string(),
    })
}

impl Herdr for CliHerdr {
    fn status(&self) -> HResult<Value> {
        self.exec_json(self.args.status())
    }

    fn worktree_create(&self, req: &WorktreeRequest) -> HResult<WorktreeCreated> {
        let v = self.exec_json(self.args.worktree_create(req))?;
        parse_worktree_created(&v)
    }

    fn worktree_remove(&self, workspace_id: &str, force: bool) -> HResult<()> {
        self.exec(self.args.worktree_remove(workspace_id, force))
            .map(|_| ())
    }

    fn pane_split(&self, pane: &str, d: Direction, cwd: Option<&Path>) -> HResult<PaneInfo> {
        let v = self.exec_json(self.args.pane_split(pane, d, cwd))?;
        result_field("pane split", &v, "pane")
    }

    fn pane_get(&self, pane: &str) -> HResult<PaneInfo> {
        let v = self.exec_json(self.args.pane_get(pane))?;
        result_field("pane get", &v, "pane")
    }

    fn pane_close(&self, pane: &str) -> HResult<()> {
        self.exec(self.args.pane_close(pane)).map(|_| ())
    }

    fn agent_start(
        &self,
        name: &str,
        kind: &str,
        pane: &str,
        args: &[String],
        timeout_ms: u64,
    ) -> HResult<AgentInfo> {
        let v = self.exec_json(self.args.agent_start(name, kind, pane, args, timeout_ms))?;
        result_field("agent start", &v, "agent")
    }

    fn agent_prompt_wait(&self, name: &str, text: &str, timeout_ms: u64) -> HResult<AgentInfo> {
        let v = self.exec_json(self.args.agent_prompt_wait(name, text, timeout_ms))?;
        result_field("agent prompt", &v, "agent")
    }

    fn agent_get(&self, name: &str) -> HResult<Option<AgentInfo>> {
        match self.exec_json(self.args.agent_get(name)) {
            Ok(v) => result_field("agent get", &v, "agent").map(Some),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn agent_read(&self, name: &str, lines: u32) -> HResult<String> {
        let v = self.exec_json(self.args.agent_read(name, lines))?;
        let read: Value = result_field("agent read", &v, "read")?;
        Ok(read
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    fn agent_send_keys(&self, name: &str, keys: &[&str]) -> HResult<()> {
        self.exec(self.args.agent_send_keys(name, keys)).map(|_| ())
    }

    fn notify(&self, title: &str, body: &str, sound: Sound) -> HResult<()> {
        self.exec(self.args.notify(title, body, sound)).map(|_| ())
    }

    fn report_metadata(&self, pane: &str, tokens: &[(&str, String)]) -> HResult<()> {
        self.exec(self.args.report_metadata(pane, tokens))
            .map(|_| ())
    }

    fn integration_status(&self) -> HResult<String> {
        self.exec(self.args.integration_status())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/herdr")
            .join(name);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    fn a() -> Args {
        Args { session: None }
    }

    #[test]
    fn session_flag_comes_first() {
        let s = Args {
            session: Some("work".into()),
        };
        assert_eq!(
            s.pane_get("w1:p1"),
            ["--session", "work", "pane", "get", "w1:p1"]
        );
        assert_eq!(a().pane_get("w1:p1"), ["pane", "get", "w1:p1"]);
    }

    #[test]
    fn worktree_create_args() {
        let req = WorktreeRequest {
            repo_path: Path::new("/r/foo"),
            branch: "muster/feat/f1",
            base: Some("origin/main"),
            path: Some(Path::new("/wt/foo/f1")),
            label: "f1",
        };
        assert_eq!(
            a().worktree_create(&req),
            [
                "worktree",
                "create",
                "--cwd",
                "/r/foo",
                "--branch",
                "muster/feat/f1",
                "--base",
                "origin/main",
                "--path",
                "/wt/foo/f1",
                "--label",
                "f1",
                "--no-focus"
            ]
        );
        let req = WorktreeRequest {
            base: None,
            path: None,
            ..req
        };
        assert!(!a().worktree_create(&req).contains(&"--base".to_string()));
    }

    #[test]
    fn agent_args() {
        assert_eq!(
            a().agent_start(
                "plan-f1",
                "claude",
                "w2:p1",
                &["--model".into(), "opus".into()],
                60000
            ),
            [
                "agent",
                "start",
                "plan-f1",
                "--kind",
                "claude",
                "--pane",
                "w2:p1",
                "--timeout",
                "60000",
                "--",
                "--model",
                "opus"
            ]
        );
        assert!(!a()
            .agent_start("x", "codex", "w1:p1", &[], 1)
            .contains(&"--".to_string()));
        assert_eq!(
            a().agent_prompt_wait("plan-f1", "Read /x.md and follow it exactly.", 600000),
            [
                "agent",
                "prompt",
                "plan-f1",
                "Read /x.md and follow it exactly.",
                "--wait",
                "--timeout",
                "600000"
            ]
        );
        assert_eq!(
            a().agent_read("plan-f1", 60),
            [
                "agent",
                "read",
                "plan-f1",
                "--source",
                "recent-unwrapped",
                "--lines",
                "60"
            ]
        );
    }

    #[test]
    fn pane_and_misc_args() {
        assert_eq!(
            a().pane_split("w2:p1", Direction::Down, None),
            [
                "pane",
                "split",
                "w2:p1",
                "--direction",
                "down",
                "--no-focus"
            ]
        );
        assert_eq!(
            a().worktree_remove("w2", true),
            ["worktree", "remove", "--workspace", "w2", "--force"]
        );
        assert_eq!(
            a().notify("f1 ready", "https://x", Sound::Done),
            [
                "notification",
                "show",
                "f1 ready",
                "--body",
                "https://x",
                "--sound",
                "done"
            ]
        );
        assert_eq!(
            a().report_metadata("w2:p1", &[("stage", "plan".into()), ("iter", "1".into())]),
            [
                "pane",
                "report-metadata",
                "w2:p1",
                "--source",
                "muster",
                "--token",
                "stage=plan",
                "--token",
                "iter=1"
            ]
        );
    }

    #[test]
    fn maps_recorded_errors() {
        let cases = [
            (
                "agent_get_not_found",
                ErrorKind::NotFound,
                "agent_not_found",
            ),
            ("agent_start_busy", ErrorKind::PaneBusy, "agent_pane_busy"),
            (
                "agent_start_bad_name",
                ErrorKind::Other,
                "invalid_agent_name",
            ),
            ("pane_close_bad", ErrorKind::NotFound, "pane_not_found"),
            (
                "worktree_remove_missing",
                ErrorKind::NotFound,
                "workspace_not_found",
            ),
        ];
        for (name, kind, code) in cases {
            let exit: i32 = fixture(&format!("{name}.exit")).trim().parse().unwrap();
            let e = map_error(
                "x",
                Some(exit),
                &fixture(&format!("{name}.stdout")),
                &fixture(&format!("{name}.stderr")),
            );
            assert_eq!((e.kind, e.code.as_str()), (kind, code), "{name}");
            assert!(!e.message.is_empty());
        }
        let e = map_error(
            "agent start",
            Some(2),
            &fixture("agent_start_bad_kind.stdout"),
            &fixture("agent_start_bad_kind.stderr"),
        );
        assert_eq!(e.kind, ErrorKind::Usage);
        assert!(e.message.contains("unsupported interactive agent kind"));
    }

    #[test]
    fn maps_documented_agent_errors() {
        for (code, kind) in [
            ("timeout", ErrorKind::Timeout),
            ("agent_prompt_stalled", ErrorKind::PromptStalled),
            ("agent_not_ready", ErrorKind::NotReady),
            ("agent_blocked", ErrorKind::Blocked),
        ] {
            let stderr = format!(r#"{{"error":{{"code":"{code}","message":"m"}},"id":"x"}}"#);
            assert_eq!(map_error("x", Some(1), "", &stderr).kind, kind, "{code}");
        }
    }

    #[test]
    fn parses_recorded_responses() {
        let v: Value = serde_json::from_str(&fixture("worktree_create.stdout")).unwrap();
        let w = parse_worktree_created(&v).unwrap();
        assert_eq!(w.workspace_id, "w2");
        assert_eq!(w.root_pane, "w2:p1");
        assert_eq!(w.branch, "muster/manual/m1");
        assert!(w.path.ends_with("muster-manual-m1"));

        let v: Value = serde_json::from_str(&fixture("pane_split.stdout")).unwrap();
        let p: PaneInfo = result_field("pane split", &v, "pane").unwrap();
        assert_eq!(p.pane_id, "w2:p2");
    }
}
