//! Phase 7: doctor reports clear failures on a machine without herdr.

mod common;

use std::process::Command;

#[test]
fn doctor_without_herdr_fails_clearly() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let cfg = root.join("cfg");
    assert!(Command::new(env!("CARGO_BIN_EXE_muster"))
        .args(["init", "--out", cfg.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    std::fs::create_dir_all(root.join("state")).unwrap();
    std::fs::write(
        cfg.join("muster.toml"),
        format!(
            "repos_dir = {:?}\nstate_dir = {:?}\nlog_dir = {:?}\n[repos.missing]\n",
            root.join("repos"),
            root.join("state"),
            root.join("logs"),
        ),
    )
    .unwrap();

    let run = |json: bool| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_muster"));
        if json {
            c.arg("--json");
        }
        c.arg("doctor")
            .env("MUSTER_CONFIG_DIR", &cfg)
            .env("HERDR_BIN", "/nonexistent/herdr")
            .env("GH_BIN", "/nonexistent/gh")
            .output()
            .unwrap()
    };

    let o = run(false);
    assert_ne!(o.status.code(), Some(0));
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.contains("FAIL herdr binary"), "{text}");
    assert!(text.contains("install herdr"), "fix hint: {text}");
    assert!(text.contains("FAIL gh"), "{text}");
    assert!(text.contains("FAIL repos_dir"), "{text}");
    assert!(text.contains("FAIL log_dir"), "{text}");
    assert!(text.contains("FAIL repo missing"), "{text}");
    assert!(text.contains("ok   state_dir"), "{text}");
    assert!(text.contains("ok   workflow feature"), "{text}");

    let o = run(true);
    assert_ne!(o.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let herdr = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "herdr binary")
        .unwrap();
    assert_eq!(herdr["ok"], false);
    assert!(herdr["fix"].as_str().unwrap().contains("HERDR_BIN"));
}

#[test]
fn doctor_with_fake_herdr_checks_server_and_integrations() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let cfg = root.join("cfg");
    assert!(Command::new(env!("CARGO_BIN_EXE_muster"))
        .args(["init", "--out", cfg.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    let o = Command::new(env!("CARGO_BIN_EXE_muster"))
        .args(["--json", "doctor"])
        .env("MUSTER_CONFIG_DIR", &cfg)
        .env("HERDR_BIN", common::fake_herdr_bin())
        .env("FAKE_HERDR_STATE", root.join("fake"))
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let check = |n: &str| {
        v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == n)
            .unwrap_or_else(|| panic!("no check {n}: {v}"))
            .clone()
    };
    assert_eq!(check("herdr server")["ok"], true);
    assert_eq!(check("herdr integration claude")["ok"], true);
}
