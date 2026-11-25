//! Scripted stand-in for the herdr CLI, for integration tests.
//!
//! Point muster at it with `HERDR_BIN=<path to fake-herdr>`.
//!
//! Environment:
//! - `FAKE_HERDR_STATE`: directory for `state.json` and `log.jsonl` (required).
//! - `FAKE_HERDR_SCENARIO`: JSON scenario file (optional).
//!
//! Scenario format:
//! ```json
//! {
//!   "agents": {
//!     "<role>": [
//!       { "write": {"plan.md": "..."},        // files relative to the run dir
//!         "write_repo": {"src/x.rs": "..."},  // uncommitted files in the worktree
//!         "commit_repo": {"src/y.rs": "..."}, // files committed in the worktree
//!         "status": "idle",                   // idle | done | blocked
//!         "error": "timeout",                 // herdr error code instead of success
//!         "exit_agent": false }               // agent vanishes after this prompt
//!     ]
//!   },
//!   "start_errors": { "<role>": "agent_not_ready" },
//!   "server_down": false
//! }
//! ```
//! Roles are agent names minus the trailing `-<run id>`. Each prompt consumes
//! the next scripted response for the role; running out is an error.
//!
//! Extra commands for tests (not part of herdr):
//! - `fake-herdr fake kill-agents`: forget every live agent (simulates a
//!   herdr server restart).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Default, Serialize, Deserialize)]
struct State {
    next_ws: u32,
    workspaces: BTreeMap<String, Workspace>,
    panes: BTreeMap<String, Pane>,
    agents: BTreeMap<String, Agent>,
    /// role -> number of prompts consumed
    consumed: BTreeMap<String, usize>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Workspace {
    path: String,
    repo: String,
    next_pane: u32,
}

#[derive(Serialize, Deserialize, Clone)]
struct Pane {
    workspace: String,
    cwd: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Agent {
    pane: String,
    kind: String,
    status: String,
}

#[derive(Default, Deserialize)]
struct Scenario {
    #[serde(default)]
    agents: BTreeMap<String, Vec<Step>>,
    #[serde(default)]
    start_errors: BTreeMap<String, String>,
    #[serde(default)]
    server_down: bool,
}

#[derive(Default, Deserialize, Clone)]
struct Step {
    #[serde(default)]
    write: BTreeMap<String, String>,
    #[serde(default)]
    write_repo: BTreeMap<String, String>,
    #[serde(default)]
    commit_repo: BTreeMap<String, String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    exit_agent: bool,
}

struct Fail {
    exit: i32,
    code: String,
    message: String,
}

fn fail(code: &str, message: impl Into<String>) -> Fail {
    Fail {
        exit: 1,
        code: code.into(),
        message: message.into(),
    }
}

fn usage(message: impl Into<String>) -> Fail {
    Fail {
        exit: 2,
        code: String::new(),
        message: message.into(),
    }
}

fn main() {
    let dir = PathBuf::from(std::env::var("FAKE_HERDR_STATE").expect("FAKE_HERDR_STATE"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut argv: Vec<String> = std::env::args().skip(1).collect();

    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("log.jsonl"))
        .unwrap();
    writeln!(log, "{}", json!(argv)).unwrap();

    if argv.first().map(String::as_str) == Some("--session") {
        argv.drain(..2.min(argv.len()));
    }

    let scenario: Scenario = match std::env::var("FAKE_HERDR_SCENARIO") {
        Ok(p) if !p.is_empty() => {
            serde_json::from_str(&std::fs::read_to_string(&p).expect("read scenario"))
                .expect("parse scenario")
        }
        _ => Scenario::default(),
    };
    let state_path = dir.join("state.json");
    let mut state: State = std::fs::read_to_string(&state_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let id = format!(
        "cli:{}",
        argv.iter().take(2).cloned().collect::<Vec<_>>().join(":")
    );
    let result = if scenario.server_down && argv.first().map(String::as_str) != Some("fake") {
        Err(fail("server_unavailable", "no herdr server is running"))
    } else {
        dispatch(&argv, &mut state, &scenario, &dir)
    };
    std::fs::write(&state_path, serde_json::to_string_pretty(&state).unwrap()).unwrap();

    match result {
        Ok(Some(v)) => {
            println!("{}", json!({"id": id, "result": v}));
        }
        Ok(None) => {}
        Err(f) if f.exit == 2 => {
            eprintln!("{}", f.message);
            std::process::exit(2);
        }
        Err(f) => {
            eprintln!(
                "{}",
                json!({"error": {"code": f.code, "message": f.message}, "id": id})
            );
            std::process::exit(f.exit);
        }
    }
}

fn flag(argv: &[String], name: &str) -> Option<String> {
    argv.iter()
        .position(|a| a == name)
        .and_then(|i| argv.get(i + 1).cloned())
}

fn has(argv: &[String], name: &str) -> bool {
    argv.iter().any(|a| a == name)
}

fn pane_json(id: &str, p: &Pane, status: &str) -> Value {
    json!({
        "pane_id": id,
        "workspace_id": p.workspace,
        "tab_id": format!("{}:t1", p.workspace),
        "agent_status": status,
        "cwd": p.cwd,
        "focused": false,
        "revision": 0,
    })
}

fn agent_json(name: &str, a: &Agent, state: &State) -> Value {
    let ws = state
        .panes
        .get(&a.pane)
        .map(|p| p.workspace.clone())
        .unwrap_or_default();
    json!({
        "name": name,
        "agent": a.kind,
        "pane_id": a.pane,
        "workspace_id": ws,
        "tab_id": format!("{ws}:t1"),
        "agent_status": a.status,
        "terminal_id": "term_fake",
        "focused": false,
        "revision": 0,
    })
}

fn role_of(name: &str) -> &str {
    name.rsplit_once('-').map(|(r, _)| r).unwrap_or(name)
}

fn valid_name(n: &str) -> bool {
    let mut c = n.chars();
    n.len() <= 32
        && matches!(c.next(), Some(x) if x.is_ascii_lowercase())
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '-' || x == '_')
}

fn git(cwd: &str, args: &[&str]) -> Result<(), Fail> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|e| fail("git", e.to_string()))?;
    if !out.status.success() {
        return Err(fail(
            "git_failed",
            format!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr)
            ),
        ));
    }
    Ok(())
}

fn write_files(base: &Path, files: &BTreeMap<String, String>) {
    for (rel, content) in files {
        let p = base.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
}

fn dispatch(
    argv: &[String],
    st: &mut State,
    sc: &Scenario,
    dir: &Path,
) -> Result<Option<Value>, Fail> {
    let a: Vec<&str> = argv.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["fake", "kill-agents"] => {
            st.agents.clear();
            Ok(None)
        }
        ["status", "server", ..] => {
            // Like the real binary, `status server --json` is not wrapped in `result`.
            println!(
                "{}",
                json!({"status": "running", "running": true, "version": "fake", "compatible": true})
            );
            Ok(None)
        }
        ["integration", "status"] => {
            println!("claude: current (v10) (/fake/.claude/hooks/herdr-agent-state.sh)");
            println!("codex: current (v3) (/fake/.codex/herdr-agent-state.sh)");
            Ok(None)
        }
        ["worktree", "create", ..] => {
            let cwd = flag(argv, "--cwd").ok_or_else(|| usage("worktree create needs --cwd"))?;
            let branch =
                flag(argv, "--branch").ok_or_else(|| usage("worktree create needs --branch"))?;
            let label = flag(argv, "--label").unwrap_or_else(|| branch.replace('/', "-"));
            let path = flag(argv, "--path").unwrap_or_else(|| {
                dir.join("worktrees")
                    .join(branch.replace('/', "-"))
                    .display()
                    .to_string()
            });
            let mut gargs = vec!["worktree", "add", "-b", &branch, &path];
            let base = flag(argv, "--base");
            if let Some(b) = &base {
                gargs.push(b);
            }
            git(&cwd, &gargs)?;
            st.next_ws += 1;
            let ws = format!("w{}", st.next_ws);
            st.workspaces.insert(
                ws.clone(),
                Workspace {
                    path: path.clone(),
                    repo: cwd.clone(),
                    next_pane: 1,
                },
            );
            let pane_id = format!("{ws}:p1");
            let pane = Pane {
                workspace: ws.clone(),
                cwd: path.clone(),
            };
            st.panes.insert(pane_id.clone(), pane.clone());
            Ok(Some(json!({
                "type": "worktree_created",
                "root_pane": pane_json(&pane_id, &pane, "unknown"),
                "workspace": {"workspace_id": ws, "label": label},
                "worktree": {"branch": branch, "path": path, "is_linked_worktree": true},
            })))
        }
        ["worktree", "remove", ..] => {
            let ws = flag(argv, "--workspace").ok_or_else(|| usage("needs --workspace"))?;
            let w = st
                .workspaces
                .remove(&ws)
                .ok_or_else(|| fail("workspace_not_found", format!("workspace {ws} not found")))?;
            let mut gargs = vec!["worktree", "remove", w.path.as_str()];
            if has(argv, "--force") {
                gargs.push("--force");
            }
            git(&w.repo, &gargs)?;
            let gone: Vec<String> = st
                .panes
                .iter()
                .filter(|(_, p)| p.workspace == ws)
                .map(|(k, _)| k.clone())
                .collect();
            for p in &gone {
                st.panes.remove(p);
            }
            st.agents.retain(|_, a| !gone.contains(&a.pane));
            Ok(Some(
                json!({"type": "worktree_removed", "workspace_id": ws, "path": w.path, "forced": has(argv, "--force")}),
            ))
        }
        ["pane", "split", pane, ..] => {
            let src = st
                .panes
                .get(*pane)
                .cloned()
                .ok_or_else(|| fail("pane_not_found", format!("pane {pane} not found")))?;
            let ws = st.workspaces.get_mut(&src.workspace).unwrap();
            ws.next_pane += 1;
            let id = format!("{}:p{}", src.workspace, ws.next_pane);
            let p = Pane {
                workspace: src.workspace.clone(),
                cwd: flag(argv, "--cwd").unwrap_or(src.cwd.clone()),
            };
            st.panes.insert(id.clone(), p.clone());
            Ok(Some(
                json!({"type": "pane_info", "pane": pane_json(&id, &p, "unknown")}),
            ))
        }
        ["pane", "get", pane] => {
            let p = st
                .panes
                .get(*pane)
                .ok_or_else(|| fail("pane_not_found", format!("pane {pane} not found")))?;
            let status = st
                .agents
                .values()
                .find(|a| a.pane == *pane)
                .map(|a| a.status.clone())
                .unwrap_or_else(|| "unknown".into());
            Ok(Some(
                json!({"type": "pane_info", "pane": pane_json(pane, p, &status)}),
            ))
        }
        ["pane", "close", pane] => {
            st.panes
                .remove(*pane)
                .ok_or_else(|| fail("pane_not_found", format!("pane {pane} not found")))?;
            st.agents.retain(|_, a| a.pane != *pane);
            Ok(None)
        }
        ["pane", "report-metadata", pane, ..] => {
            if !st.panes.contains_key(*pane) {
                return Err(fail("pane_not_found", format!("pane {pane} not found")));
            }
            Ok(None)
        }
        ["notification", "show", ..] => Ok(Some(
            json!({"type": "notification_show", "shown": false, "reason": "disabled"}),
        )),
        ["agent", "start", name, ..] => {
            if !valid_name(name) {
                return Err(fail(
                    "invalid_agent_name",
                    "agent name must start with a lowercase letter",
                ));
            }
            let kind = flag(argv, "--kind").ok_or_else(|| usage("needs --kind"))?;
            let pane = flag(argv, "--pane").ok_or_else(|| usage("needs --pane"))?;
            if !st.panes.contains_key(&pane) {
                return Err(fail("pane_not_found", format!("pane {pane} not found")));
            }
            if st.agents.values().any(|a| a.pane == pane) {
                return Err(fail(
                    "agent_pane_busy",
                    format!("agent target pane {pane} is not an available shell"),
                ));
            }
            if st.agents.contains_key(*name) {
                return Err(fail(
                    "agent_name_taken",
                    format!("agent name {name} is in use"),
                ));
            }
            if let Some(code) = sc.start_errors.get(role_of(name)) {
                return Err(fail(code, format!("scripted start error for {name}")));
            }
            let agent = Agent {
                pane,
                kind,
                status: "idle".into(),
            };
            st.agents.insert(name.to_string(), agent.clone());
            let args: Vec<&String> = argv
                .iter()
                .skip_while(|x| x.as_str() != "--")
                .skip(1)
                .collect();
            Ok(Some(
                json!({"type": "agent_started", "agent": agent_json(name, &agent, st), "argv": args}),
            ))
        }
        ["agent", "get", name] => {
            let a = st
                .agents
                .get(*name)
                .ok_or_else(|| fail("agent_not_found", format!("agent target {name} not found")))?;
            Ok(Some(
                json!({"type": "agent_info", "agent": agent_json(name, a, st)}),
            ))
        }
        ["agent", "read", name, ..] => {
            if !st.agents.contains_key(*name) {
                return Err(fail(
                    "agent_not_found",
                    format!("agent target {name} not found"),
                ));
            }
            Ok(Some(
                json!({"type": "pane_read", "read": {"text": format!("fake output of {name}\n"), "pane_id": "", "workspace_id": "", "tab_id": "", "source": "recent-unwrapped", "format": "text", "revision": 0, "truncated": false}}),
            ))
        }
        ["agent", "send-keys", name, ..] => {
            if !st.agents.contains_key(*name) {
                return Err(fail(
                    "agent_not_found",
                    format!("agent target {name} not found"),
                ));
            }
            Ok(None)
        }
        ["agent", "prompt", name, text, ..] => {
            let agent =
                st.agents.get(*name).cloned().ok_or_else(|| {
                    fail("agent_not_found", format!("agent target {name} not found"))
                })?;
            if agent.status == "blocked" {
                return Err(fail("agent_blocked", "agent is waiting on a dialog"));
            }
            let role = role_of(name).to_string();
            let n = *st.consumed.get(&role).unwrap_or(&0);
            let step = sc
                .agents
                .get(&role)
                .and_then(|s| s.get(n))
                .cloned()
                .ok_or_else(|| {
                    fail(
                        "fake_exhausted",
                        format!("no scripted response #{n} for role {role}"),
                    )
                })?;
            st.consumed.insert(role, n + 1);

            let prompt_path = text
                .strip_prefix("Read ")
                .and_then(|r| r.split(" and follow").next())
                .ok_or_else(|| {
                    fail("fake_bad_prompt", format!("unexpected prompt text: {text}"))
                })?;
            let prompt_path = Path::new(prompt_path);
            if !prompt_path.is_file() {
                return Err(fail(
                    "fake_bad_prompt",
                    format!("prompt file missing: {}", prompt_path.display()),
                ));
            }
            let run_dir = prompt_path.parent().and_then(Path::parent).unwrap();
            let worktree = st
                .panes
                .get(&agent.pane)
                .map(|p| p.cwd.clone())
                .unwrap_or_default();

            write_files(run_dir, &step.write);
            write_files(Path::new(&worktree), &step.write_repo);
            if !step.commit_repo.is_empty() {
                write_files(Path::new(&worktree), &step.commit_repo);
                let mut args = vec!["add", "--"];
                args.extend(step.commit_repo.keys().map(String::as_str));
                git(&worktree, &args)?;
                git(
                    &worktree,
                    &["commit", "-q", "-m", &format!("{name}: scripted change")],
                )?;
            }
            if step.exit_agent {
                st.agents.remove(*name);
            }
            if let Some(code) = &step.error {
                return Err(fail(code, format!("scripted {code} for {name}")));
            }
            let status = step.status.clone().unwrap_or_else(|| "idle".into());
            let mut agent = agent;
            agent.status = status;
            if let Some(a) = st.agents.get_mut(*name) {
                a.status = agent.status.clone();
            }
            Ok(Some(
                json!({"type": "agent_prompted", "agent": agent_json(name, &agent, st)}),
            ))
        }
        _ => Err(usage(format!(
            "fake-herdr: unsupported command: {}",
            argv.join(" ")
        ))),
    }
}
