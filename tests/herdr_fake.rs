//! Phase 2: drive the real `CliHerdr` against fake-herdr.

mod common;

use muster::herdr::{CliHerdr, Direction, ErrorKind, Herdr, Sound, WorktreeRequest};

#[test]
fn cli_herdr_against_fake() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = common::make_repo(tmp.path(), "sandbox");
    let state = tmp.path().join("fake");
    let scenario = tmp.path().join("scenario.json");
    std::fs::write(
        &scenario,
        r##"{"agents": {"planner": [{"write": {"plan.md": "# Plan\n"}}, {"error": "timeout"}]}}"##,
    )
    .unwrap();
    std::env::set_var("FAKE_HERDR_STATE", &state);
    std::env::set_var("FAKE_HERDR_SCENARIO", &scenario);

    let h = CliHerdr::new(common::fake_herdr_bin(), Some("test".into()), false);
    assert_eq!(h.status().unwrap()["running"], true);

    let wt_path = tmp.path().join("wt/m1");
    let wt = h
        .worktree_create(&WorktreeRequest {
            repo_path: &repo,
            branch: "muster/manual/m1",
            base: Some("origin/main"),
            path: Some(&wt_path),
            label: "m1",
        })
        .unwrap();
    assert_eq!(wt.path, wt_path);
    assert!(wt_path.join("README.md").is_file());

    let pane = h.pane_split(&wt.root_pane, Direction::Right, None).unwrap();
    assert_ne!(pane.pane_id, wt.root_pane);
    assert_eq!(
        h.pane_get(&pane.pane_id).unwrap().workspace_id,
        wt.workspace_id
    );

    let agent = h
        .agent_start(
            "planner-m1",
            "claude",
            &wt.root_pane,
            &["--model".into(), "opus".into()],
            30000,
        )
        .unwrap();
    assert_eq!(agent.pane_id, wt.root_pane);

    // A second agent in the same pane is refused.
    let busy = h
        .agent_start("other-m1", "claude", &wt.root_pane, &[], 1000)
        .unwrap_err();
    assert_eq!(busy.kind, ErrorKind::PaneBusy);

    let run_dir = wt_path.join(".muster/runs/m1");
    std::fs::create_dir_all(run_dir.join("prompts")).unwrap();
    let prompt = run_dir.join("prompts/plan-1.md");
    std::fs::write(&prompt, "write the plan").unwrap();
    let text = format!("Read {} and follow it exactly.", prompt.display());
    let settled = h.agent_prompt_wait("planner-m1", &text, 600000).unwrap();
    assert!(settled.is_settled());
    assert_eq!(
        std::fs::read_to_string(run_dir.join("plan.md")).unwrap(),
        "# Plan\n"
    );

    let err = h.agent_prompt_wait("planner-m1", &text, 1000).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Timeout);
    assert!(
        err.raw.stderr.contains("\"timeout\""),
        "raw JSON is preserved"
    );

    assert!(h.agent_get("planner-m1").unwrap().is_some());
    assert!(h.agent_get("nobody-m1").unwrap().is_none());
    assert!(h
        .agent_read("planner-m1", 20)
        .unwrap()
        .contains("planner-m1"));
    h.agent_send_keys("planner-m1", &["ctrl+c"]).unwrap();
    h.notify("m1 ready", "body", Sound::Done).unwrap();
    h.report_metadata(&pane.pane_id, &[("stage", "plan".into())])
        .unwrap();
    assert!(h.integration_status().unwrap().contains("claude: current"));

    h.pane_close(&pane.pane_id).unwrap();
    assert_eq!(
        h.pane_get(&pane.pane_id).unwrap_err().kind,
        ErrorKind::NotFound
    );
    h.worktree_remove(&wt.workspace_id, true).unwrap();
    assert!(!wt_path.exists());
    assert_eq!(
        h.worktree_remove(&wt.workspace_id, false).unwrap_err().kind,
        ErrorKind::NotFound
    );

    // Every call carried the session flag.
    let log = common::herdr_log(&state);
    assert!(log.len() > 10);
    assert!(log
        .iter()
        .all(|argv| argv[0] == "--session" && argv[1] == "test"));
}

#[test]
fn missing_binary_is_spawn_error() {
    let h = CliHerdr::new("/nonexistent/herdr", None, false);
    assert_eq!(h.status().unwrap_err().kind, ErrorKind::Spawn);
}
