//! Phases 4 and 5: the engine end to end, through the real binary, with
//! fake-herdr as herdr, a fake `gh`, and a local bare repo as the remote.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};

struct Env {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    cfg: PathBuf,
    repo: PathBuf,
    fake: PathBuf,
    scenario: PathBuf,
}

const APPROVE: &str = "VERDICT: APPROVE\n\n## Issues\n\nNone.\n";
const CHANGES: &str = "VERDICT: CHANGES\n\n## Issues\n\n1. src/lib.rs: add a test.\n";

impl Env {
    fn new() -> Env {
        Self::with_config("")
    }

    fn with_config(extra: &str) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let repo = common::make_repo(&root, "sandbox");
        let cfg = root.join("cfg");
        let o = Command::new(env!("CARGO_BIN_EXE_muster"))
            .args(["init", "--out", cfg.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(o.status.success());
        std::fs::write(
            cfg.join("muster.toml"),
            format!(
                "repos_dir = {:?}\nstate_dir = {:?}\nlog_dir = {:?}\nworktrees_dir = {:?}\nmax_concurrent_runs = 2\n{extra}\n",
                root.join("repos"),
                root.join("state"),
                root.join("logs"),
                root.join("worktrees"),
            ),
        )
        .unwrap();
        // Fake gh: logs its args and the PR body, prints a PR URL.
        let gh = root.join("gh");
        std::fs::write(
            &gh,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > {log}/gh.args\nwhile [ $# -gt 0 ]; do if [ \"$1\" = --body-file ]; then cp \"$2\" {log}/gh.body; fi; shift; done\necho https://github.com/o/sandbox/pull/7\n",
                log = root.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        Env {
            fake: root.join("fake"),
            scenario: root.join("scenario.json"),
            _tmp: tmp,
            root,
            cfg,
            repo,
        }
    }

    fn scenario(&self, v: Value) {
        std::fs::write(&self.scenario, v.to_string()).unwrap();
    }

    fn muster_in(&self, args: &[&str], stdin: Option<&str>) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_muster"));
        c.args(args)
            .env("MUSTER_CONFIG_DIR", &self.cfg)
            .env("HERDR_BIN", common::fake_herdr_bin())
            .env("GH_BIN", self.root.join("gh"))
            .env("ENTIRE_BIN", self.root.join("no-entire"))
            .env("FAKE_HERDR_STATE", &self.fake)
            .env("FAKE_HERDR_SCENARIO", &self.scenario)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().unwrap();
        if let Some(s) = stdin {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
        }
        drop(child.stdin.take());
        child.wait_with_output().unwrap()
    }

    fn muster(&self, args: &[&str]) -> Output {
        self.muster_in(args, None)
    }

    fn start(&self, workflow: &str, extra: &[&str]) -> Output {
        let mut args = vec!["start", workflow, "--repo", "sandbox", "--brief-stdin"];
        args.extend_from_slice(extra);
        self.muster_in(&args, Some("# Make the sandbox better\n\nDetails here.\n"))
    }

    fn state(&self, id: &str) -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(self.root.join("state").join(format!("{id}.json"))).unwrap(),
        )
        .unwrap()
    }

    fn worktree(&self, id: &str) -> PathBuf {
        PathBuf::from(self.state(id)["worktree_path"].as_str().unwrap())
    }

    fn run_dir(&self, id: &str) -> PathBuf {
        self.worktree(id).join(".muster/runs").join(id)
    }

    fn herdr_calls(&self, prefix: &[&str]) -> Vec<Vec<String>> {
        common::herdr_log(&self.fake)
            .into_iter()
            .filter(|a| a.iter().zip(prefix).all(|(x, y)| x == y) && a.len() >= prefix.len())
            .collect()
    }
}

fn out(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn happy_agents() -> Value {
    json!({
        "planner": [{"write": {"plan.md": "# Plan\n\n1. Do it.\n"}}],
        "builder": [{"commit_repo": {"src/lib.rs": "pub fn x() {}\n"}, "write": {"build-report.md": "## Changes\n\nsrc/lib.rs\n"}}],
        "reviewer": [{"write": {"review.md": APPROVE}}]
    })
}

#[test]
fn happy_path_opens_pr() {
    let e = Env::new();
    e.scenario(json!({"agents": happy_agents()}));
    let o = e.start("feature", &[]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));

    let st = e.state("f1");
    assert_eq!(st["status"], "done");
    assert_eq!(st["pr_url"], "https://github.com/o/sandbox/pull/7");
    assert_eq!(st["branch"], "muster/feat/f1");

    // Paper trail on the branch.
    let wt = e.worktree("f1");
    assert!(wt.starts_with(e.root.join("worktrees")));
    let tracked = common::git(&wt, &["ls-files", ".muster"]);
    for f in [
        "brief.md",
        "plan.md",
        "build-report.md",
        "review.md",
        "prompts/plan-1.md",
        "prompts/build-1.md",
        "prompts/review-1.md",
        "config/workflows/feature.toml",
        "config/prompts/builder.md",
    ] {
        assert!(
            tracked.contains(&format!(".muster/runs/f1/{f}")),
            "{f} not committed:\n{tracked}"
        );
    }
    let prompt = std::fs::read_to_string(e.run_dir("f1").join("prompts/build-1.md")).unwrap();
    assert!(prompt.contains("iteration 1"));
    assert!(prompt.contains("previous review: `none`"));
    assert!(!prompt.contains("{{"));

    // Pushed to the remote.
    let remote = e.root.join("remotes/sandbox.git");
    assert!(
        common::git(&remote, &["branch", "--list", "muster/feat/f1"]).contains("muster/feat/f1")
    );
    assert!(!common::git(&wt, &["status", "--porcelain"]).contains("M "));

    // PR title from the brief, body from pr_body_from plus the footer.
    let args = std::fs::read_to_string(e.root.join("gh.args")).unwrap();
    assert!(args.contains("Make the sandbox better"));
    assert!(args.contains("--base\nmain\n--head\nmuster/feat/f1"));
    assert!(!args.contains("--draft"));
    let body = std::fs::read_to_string(e.root.join("gh.body")).unwrap();
    assert!(body.contains("## plan.md") && body.contains("1. Do it."));
    assert!(body.contains("## review.md") && body.contains("VERDICT: APPROVE"));
    assert!(body.contains("muster run `f1`"));

    // herdr usage: one agent per role, each in its own pane; metadata and a
    // done notification.
    let starts = e.herdr_calls(&["agent", "start"]);
    let names: Vec<&str> = starts.iter().map(|a| a[2].as_str()).collect();
    assert_eq!(names, ["planner-f1", "builder-f1", "reviewer-f1"]);
    let panes: std::collections::BTreeSet<_> = st["agents"]
        .as_object()
        .unwrap()
        .values()
        .map(|a| a["pane_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(panes.len(), 3);
    assert_eq!(e.herdr_calls(&["pane", "split"]).len(), 2);
    assert_eq!(e.herdr_calls(&["pane", "report-metadata"]).len(), 3);
    let notes = e.herdr_calls(&["notification", "show"]);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0][2], "f1 ready");

    // status
    let o = e.muster(&["--json", "status"]);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["runs"][0]["id"], "f1");
    assert_eq!(v["runs"][0]["status"], "done");
    let o = e.muster(&["status", "f1"]);
    assert!(
        out(&o).contains("config    unchanged since start"),
        "{}",
        out(&o)
    );

    // clean
    let o = e.muster(&["clean", "f1", "--delete-branch"]);
    assert!(o.status.success(), "{}", out(&o));
    assert!(!wt.exists());
    assert!(common::git(&e.repo, &["branch", "--list", "muster/feat/f1"]).is_empty());
    assert!(e.state("f1")["cleaned_at"].is_string());
}

#[test]
fn review_loop_approves_on_iteration_2() {
    let e = Env::new();
    e.scenario(json!({"agents": {
        "planner": [{"write": {"plan.md": "# Plan\n"}}],
        "builder": [
            {"write": {"build-report.md": "first\n"}},
            {"write": {"build-report.md": "second\n"}}
        ],
        "reviewer": [{"write": {"review.md": CHANGES}}, {"write": {"review.md": APPROVE}}]
    }}));
    let o = e.start("feature", &[]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    let rd = e.run_dir("f1");
    for f in [
        "prompts/build-2.md",
        "prompts/review-2.md",
        "build-report-1.md",
        "review-1.md",
    ] {
        assert!(rd.join(f).is_file(), "{f}");
    }
    assert_eq!(
        std::fs::read_to_string(rd.join("build-report.md")).unwrap(),
        "second\n"
    );
    assert!(std::fs::read_to_string(rd.join("review-1.md"))
        .unwrap()
        .starts_with("VERDICT: CHANGES"));
    // Build 2 got the iteration-1 review; review 2 got the archived copy.
    let b2 = std::fs::read_to_string(rd.join("prompts/build-2.md")).unwrap();
    assert!(
        b2.contains(&format!(
            "previous review: `{}`",
            rd.join("review.md").display()
        )),
        "{b2}"
    );
    let r2 = std::fs::read_to_string(rd.join("prompts/review-2.md")).unwrap();
    assert!(
        r2.contains(&rd.join("review-1.md").display().to_string()),
        "{r2}"
    );
    // The same builder agent was reused, not restarted.
    assert_eq!(e.herdr_calls(&["agent", "start", "builder-f1"]).len(), 1);
    assert_eq!(e.state("f1")["cursor"]["iteration"], 2);
}

#[test]
fn review_loop_escalates_at_max_iterations() {
    let e = Env::new();
    e.scenario(json!({"agents": {
        "planner": [{"write": {"plan.md": "# Plan\n"}}],
        "builder": [{"write": {"build-report.md": "1\n"}}, {"write": {"build-report.md": "2\n"}}],
        "reviewer": [{"write": {"review.md": CHANGES}}, {"write": {"review.md": CHANGES}}]
    }}));
    let o = e.start("feature", &[]);
    assert_eq!(o.status.code(), Some(3), "{}", out(&o));
    let st = e.state("f1");
    assert_eq!(st["status"], "needs_human");
    assert!(st["reason"]
        .as_str()
        .unwrap()
        .contains("max_iterations = 2"));
    let n = e.herdr_calls(&["notification", "show"]);
    assert_eq!(n.last().unwrap()[2], "f1: review needs you");
    assert!(out(&o).contains("muster resume f1"));
}

#[test]
fn missing_artifact_escalates_then_resume_reprompts() {
    let e = Env::new();
    e.scenario(json!({"agents": {
        "planner": [{}, {"write": {"plan.md": "# Plan\n"}}],
        "builder": [{"write": {"build-report.md": "ok\n"}}],
        "reviewer": [{"write": {"review.md": APPROVE}}]
    }}));
    let o = e.start("feature", &[]);
    assert_eq!(o.status.code(), Some(3), "{}", out(&o));
    assert!(e.state("f1")["reason"]
        .as_str()
        .unwrap()
        .contains("artifact missing"));

    // The human leaves notes; resume prompts the planner again and mentions them.
    std::fs::write(e.run_dir("f1").join("human-notes.md"), "Use approach B.\n").unwrap();
    let o = e.muster(&["resume", "f1"]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    let prompts = e.herdr_calls(&["agent", "prompt", "planner-f1"]);
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1][3].contains("human-notes.md"));
    assert_eq!(e.state("f1")["status"], "done");
}

#[test]
fn blocked_artifact_escalates() {
    let e = Env::new();
    e.scenario(json!({"agents": {
        "planner": [{"write": {"plan.md": "# Plan\n\n## Blocked\n\nShould this use Postgres or SQLite?\n"}}]
    }}));
    let o = e.start("feature", &[]);
    assert_eq!(o.status.code(), Some(3), "{}", out(&o));
    let reason = e.state("f1")["reason"].as_str().unwrap().to_string();
    assert!(
        reason.contains("Blocked") && reason.contains("Postgres or SQLite"),
        "{reason}"
    );
}

#[test]
fn timeout_escalates_and_resume_accepts_hand_finished_stage() {
    let e = Env::new();
    e.scenario(json!({"agents": {
        "planner": [{"error": "timeout"}],
        "builder": [{"write": {"build-report.md": "ok\n"}}],
        "reviewer": [{"write": {"review.md": APPROVE}}]
    }}));
    let o = e.start("feature", &[]);
    assert_eq!(o.status.code(), Some(3), "{}", out(&o));
    let st = e.state("f1");
    assert!(st["reason"].as_str().unwrap().contains("timeout"));
    let err = st["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|ev| ev["kind"] == "error")
        .unwrap();
    assert!(err["detail"]["stderr"]
        .as_str()
        .unwrap()
        .contains("\"timeout\""));

    // The human finishes the plan by hand; resume must not prompt the planner again.
    std::fs::write(e.run_dir("f1").join("plan.md"), "# Plan by hand\n").unwrap();
    let o = e.muster(&["resume", "f1"]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    assert_eq!(e.herdr_calls(&["agent", "prompt", "planner-f1"]).len(), 1);
    assert_eq!(e.state("f1")["status"], "done");
}

#[test]
fn resume_restarts_vanished_agents() {
    let e = Env::new();
    e.scenario(json!({"agents": {
        "planner": [{"write": {"plan.md": "# Plan\n"}}],
        "builder": [{"error": "agent_prompt_stalled"}, {"write": {"build-report.md": "ok\n"}}],
        "reviewer": [{"write": {"review.md": APPROVE}}]
    }}));
    assert_eq!(e.start("feature", &[]).status.code(), Some(3));
    // A herdr server restart: every agent is gone.
    Command::new(common::fake_herdr_bin())
        .args(["fake", "kill-agents"])
        .env("FAKE_HERDR_STATE", &e.fake)
        .status()
        .unwrap();
    let o = e.muster(&["resume", "f1"]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    // The builder was restarted in its own pane, not a new one.
    let starts = e.herdr_calls(&["agent", "start", "builder-f1"]);
    assert_eq!(starts.len(), 2);
    let pane = |a: &Vec<String>| a[a.iter().position(|x| x == "--pane").unwrap() + 1].clone();
    assert_eq!(pane(&starts[0]), pane(&starts[1]));
}

#[test]
fn agent_not_ready_escalates() {
    let e = Env::new();
    e.scenario(json!({"start_errors": {"planner": "agent_not_ready"}}));
    let o = e.start("feature", &[]);
    assert_eq!(o.status.code(), Some(3), "{}", out(&o));
    assert!(e.state("f1")["reason"]
        .as_str()
        .unwrap()
        .contains("agent_not_ready"));
}

#[test]
fn read_only_stage_must_not_touch_repo() {
    let e = Env::new();
    e.scenario(json!({"agents": {
        "planner": [{"write": {"plan.md": "# Plan\n"}, "write_repo": {"src/sneaky.rs": "x"}}]
    }}));
    let o = e.start("feature", &[]);
    assert_eq!(o.status.code(), Some(3), "{}", out(&o));
    let reason = e.state("f1")["reason"].as_str().unwrap().to_string();
    assert!(
        reason.contains("read-only") && reason.contains("src/sneaky.rs"),
        "{reason}"
    );
}

#[test]
fn concurrency_limit() {
    let e = Env::with_config("");
    std::fs::write(
        e.cfg.join("muster.toml"),
        std::fs::read_to_string(e.cfg.join("muster.toml"))
            .unwrap()
            .replace("max_concurrent_runs = 2", "max_concurrent_runs = 1"),
    )
    .unwrap();
    // Another live process is driving run x1: its state says running and its
    // lock is held.
    let state_dir = e.root.join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    e.scenario(json!({"agents": happy_agents()}));
    assert_eq!(
        e.start("feature", &["--run-id", "x1"]).status.code(),
        Some(0)
    );
    let mut st = e.state("x1");
    st["status"] = json!("running");
    std::fs::write(state_dir.join("x1.json"), st.to_string()).unwrap();
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .open(state_dir.join("x1.lock"))
        .unwrap();
    lock.try_lock().unwrap();

    let o = e.start("feature", &[]);
    assert_eq!(o.status.code(), Some(5), "{}", out(&o));
    assert!(out(&o).contains("concurrency limit"));

    // Once the lock is released, x1 is just a stale record and doesn't count.
    drop(lock);
    e.scenario(json!({"agents": {
        "planner": [{"write": {"plan.md": "p"}}, {"write": {"plan.md": "p"}}],
        "builder": [{"write": {"build-report.md": "b"}}, {"write": {"build-report.md": "b"}}],
        "reviewer": [{"write": {"review.md": APPROVE}}, {"write": {"review.md": APPROVE}}]
    }}));
    std::fs::remove_dir_all(&e.fake).unwrap();
    let o = e.start("feature", &[]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    let o = e.muster(&["status"]);
    assert!(out(&o).contains("running (stale)"), "{}", out(&o));
}

fn write_optional_workflow(cfg: &Path) {
    std::fs::write(
        cfg.join("workflows/docs.toml"),
        r#"
[workflow]
name = "docs"
branch_prefix = "muster/docs"

[[stage]]
id = "plan"
role = "planner"
kind = "claude"
produces = "plan.md"

[[stage]]
id = "tutorial"
role = "builder"
kind = "claude"
produces = "tutorial.md"
optional = true

[finish]
open_pr = false
"#,
    )
    .unwrap();
}

#[test]
fn skip_optional_stage() {
    let e = Env::new();
    write_optional_workflow(&e.cfg);
    e.scenario(json!({"agents": {"planner": [{"write": {"plan.md": "# Plan\n"}}]}}));
    let o = e.start("docs", &["--skip", "tutorial"]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    let st = e.state("d1");
    assert_eq!(st["status"], "done");
    assert!(st["pr_url"].is_null());
    assert!(st["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|ev| ev["kind"] == "skipped" && ev["stage"] == "tutorial"));
    assert!(e.herdr_calls(&["agent", "start", "builder-d1"]).is_empty());
    assert!(!e.root.join("gh.args").exists());

    // Skipping a required stage is a usage error.
    let o = e.start("docs", &["--skip", "plan"]);
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
}

#[test]
fn live_config_change_does_not_affect_resumed_run() {
    let e = Env::new();
    e.scenario(json!({"agents": {
        "planner": [{}],
        "builder": [{"write": {"build-report.md": "ok\n"}}],
        "reviewer": [{"write": {"review.md": APPROVE}}]
    }}));
    assert_eq!(e.start("feature", &[]).status.code(), Some(3));

    // ArgoCD syncs a new config while the run waits for a human: the builder
    // prompt changes and the review loop is removed.
    let p = e.cfg.join("prompts/builder.md");
    std::fs::write(
        &p,
        format!("{}\nLIVE CHANGE\n", std::fs::read_to_string(&p).unwrap()),
    )
    .unwrap();
    let w = e.cfg.join("workflows/feature.toml");
    std::fs::write(
        &w,
        std::fs::read_to_string(&w)
            .unwrap()
            .replace("max_iterations = 2", "max_iterations = 5"),
    )
    .unwrap();
    let o = e.muster(&["status", "f1"]);
    assert!(out(&o).contains("live config has changed"), "{}", out(&o));

    std::fs::write(e.run_dir("f1").join("plan.md"), "# Plan\n").unwrap();
    let o = e.muster(&["resume", "f1"]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    let build = std::fs::read_to_string(e.run_dir("f1").join("prompts/build-1.md")).unwrap();
    assert!(!build.contains("LIVE CHANGE"));
    let snap =
        std::fs::read_to_string(e.run_dir("f1").join("config/workflows/feature.toml")).unwrap();
    assert!(snap.contains("max_iterations = 2"));
}

#[test]
fn dirty_worktree_gets_leftover_commit() {
    let e = Env::new();
    e.scenario(json!({"agents": {
        "planner": [{"write": {"plan.md": "# Plan\n"}}],
        "builder": [{"write": {"build-report.md": "ok\n"}, "write_repo": {"src/forgot.rs": "// uncommitted\n"}}],
        "reviewer": [{"write": {"review.md": APPROVE}}]
    }}));
    // The reviewer is read-only, and the builder's leftover file predates it,
    // so the read-only check must not blame the reviewer for it.
    let o = e.start("feature", &[]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    let wt = e.worktree("f1");
    let log = common::git(&wt, &["log", "--format=%s"]);
    assert!(
        log.contains("muster: uncommitted changes after review"),
        "{log}"
    );
    assert!(common::git(&wt, &["status", "--porcelain"]).is_empty());
    assert!(common::git(&wt, &["show", "--stat", "HEAD"]).contains("src/forgot.rs"));
    let st = e.state("f1");
    assert!(st["events"].as_array().unwrap().iter().any(
        |ev| ev["kind"] == "warning" && ev["message"].as_str().unwrap().contains("uncommitted")
    ));
}

#[test]
fn detach_prints_run_id_and_runs_in_background() {
    let e = Env::new();
    e.scenario(json!({"agents": happy_agents()}));
    let o = e.muster_in(
        &[
            "--json",
            "start",
            "feature",
            "--repo",
            "sandbox",
            "--brief-stdin",
            "--detach",
        ],
        Some("Detached brief\n"),
    );
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["run_id"], "f1");
    let log = PathBuf::from(v["log"].as_str().unwrap());
    // Wait for the background run to finish (bounded).
    for _ in 0..200 {
        if e.root.join("state/f1.json").exists() && e.state("f1")["status"] == "done" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(
        e.state("f1")["status"],
        "done",
        "log:\n{}",
        std::fs::read_to_string(&log).unwrap_or_default()
    );
    assert!(std::fs::read_to_string(e.run_dir("f1").join("brief.md"))
        .unwrap()
        .contains("Detached brief"));
}

#[test]
fn resume_detach_runs_in_background() {
    let e = Env::new();
    e.scenario(json!({"agents": {
        "planner": [{}, {"write": {"plan.md": "# Plan\n"}}],
        "builder": [{"write": {"build-report.md": "ok\n"}}],
        "reviewer": [{"write": {"review.md": APPROVE}}]
    }}));
    assert_eq!(e.start("feature", &[]).status.code(), Some(3));
    let t = std::time::Instant::now();
    let o = e.muster(&["--json", "resume", "f1", "--detach"]);
    assert!(t.elapsed() < std::time::Duration::from_secs(10));
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["run_id"], "f1");
    let log = PathBuf::from(v["log"].as_str().unwrap());
    assert!(log.ends_with("f1.log"));
    for _ in 0..200 {
        if e.state("f1")["status"] == "done" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(
        e.state("f1")["status"],
        "done",
        "log:\n{}",
        std::fs::read_to_string(&log).unwrap_or_default()
    );
    assert_eq!(e.herdr_calls(&["agent", "prompt", "planner-f1"]).len(), 2);
}

#[test]
fn resume_detach_unknown_run_fails_synchronously() {
    let e = Env::new();
    e.scenario(json!({}));
    let o = e.muster(&["resume", "f9", "--detach"]);
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    assert!(!e.root.join("logs/f9.log").exists());
    let o = e.muster(&["--json", "resume", "f9", "--detach"]);
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["exit_code"], 2);
    assert!(!e.root.join("logs/f9.log").exists());
}

#[test]
fn resume_detach_locked_run_fails_synchronously() {
    let e = Env::new();
    e.scenario(json!({"agents": {"planner": [{}]}}));
    assert_eq!(e.start("feature", &[]).status.code(), Some(3));
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .open(e.root.join("state/f1.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let o = e.muster(&["resume", "f1", "--detach"]);
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    assert!(out(&o).contains("another muster process"), "{}", out(&o));
    assert!(!e.root.join("logs/f1.log").exists());
}

#[test]
fn clean_refuses_running_run_without_force() {
    let e = Env::new();
    e.scenario(json!({"agents": {"planner": [{}]}}));
    assert_eq!(e.start("feature", &[]).status.code(), Some(3));
    let state_dir = e.root.join("state");
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .open(state_dir.join("f1.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let o = e.muster(&["clean", "f1"]);
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    drop(lock);
    let o = e.muster(&["clean", "f1", "--force"]);
    assert!(o.status.success(), "{}", out(&o));
    let o = e.muster(&["resume", "f1"]);
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
}

#[test]
fn usage_errors() {
    let e = Env::new();
    e.scenario(json!({}));
    let o = e.muster_in(
        &["start", "feature", "--repo", "nope", "--brief-stdin"],
        Some("x"),
    );
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    let o = e.muster_in(
        &["start", "feature", "--repo", "sandbox", "--brief-stdin"],
        Some("  \n"),
    );
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    let o = e.muster_in(
        &["start", "nosuch", "--repo", "sandbox", "--brief-stdin"],
        Some("x"),
    );
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    let o = e.muster(&["resume", "f9"]);
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    let o = e.muster(&["start", "feature", "--repo", "sandbox"]);
    assert_eq!(o.status.code(), Some(2), "brief is required");
}

#[test]
fn inline_brief_starts_run() {
    let e = Env::new();
    e.scenario(json!({"agents": happy_agents()}));
    let text = "Fix the flaky retry test\n\nMore detail.";
    let o = e.muster(&["start", "feature", "--repo", "sandbox", "--brief", text]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    assert_eq!(
        std::fs::read_to_string(e.run_dir("f1").join("brief.md")).unwrap(),
        text
    );
    let tracked = common::git(&e.worktree("f1"), &["ls-files", ".muster"]);
    assert!(tracked.contains(".muster/runs/f1/brief.md"), "{tracked}");
    let args = std::fs::read_to_string(e.root.join("gh.args")).unwrap();
    assert!(args.contains("Fix the flaky retry test"), "{args}");
    assert!(!args.contains("More detail."), "{args}");
    let o = e.muster(&["--json", "status", "f1"]);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ok"], true, "{}", out(&o));
    assert_eq!(v["status"], "done");
    assert!(v["run_dir"].is_string());
}

#[test]
fn inline_brief_detach() {
    let e = Env::new();
    e.scenario(json!({"agents": happy_agents()}));
    let o = e.muster(&[
        "--json",
        "start",
        "feature",
        "--repo",
        "sandbox",
        "--brief",
        "Detached inline brief",
        "--detach",
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["run_id"], "f1");
    let log = PathBuf::from(v["log"].as_str().unwrap());
    for _ in 0..200 {
        if e.root.join("state/f1.json").exists() && e.state("f1")["status"] == "done" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(
        e.state("f1")["status"],
        "done",
        "log:\n{}",
        std::fs::read_to_string(&log).unwrap_or_default()
    );
    assert!(std::fs::read_to_string(e.run_dir("f1").join("brief.md"))
        .unwrap()
        .contains("Detached inline brief"));
    assert!(log.with_file_name("f1.brief.md").exists());
}

#[test]
fn brief_source_conflicts_exit_2() {
    let e = Env::new();
    e.scenario(json!({}));
    let file = e.root.join("some-brief.md");
    std::fs::write(&file, "x").unwrap();
    let file = file.to_str().unwrap();
    let base = ["start", "feature", "--repo", "sandbox"];
    let cases: [&[&str]; 3] = [
        &["--brief", "x", "--brief-file", file],
        &["--brief", "x", "--brief-stdin"],
        &["--brief-file", file, "--brief-stdin"],
    ];
    for extra in cases {
        let args: Vec<&str> = base.iter().chain(extra.iter()).copied().collect();
        let o = e.muster_in(&args, Some("x"));
        assert_eq!(o.status.code(), Some(2), "{extra:?}: {}", out(&o));
        assert!(
            String::from_utf8_lossy(&o.stderr).contains("cannot be used with"),
            "{extra:?}: {}",
            out(&o)
        );
    }
    for empty in ["", "   "] {
        let o = e.muster(&["start", "feature", "--repo", "sandbox", "--brief", empty]);
        assert_eq!(o.status.code(), Some(2), "{}", out(&o));
        assert!(out(&o).contains("the brief is empty"), "{}", out(&o));
    }
}
