//! Phase 1: init, validate and list-workflows through the real binary.

use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::process::{Command, Output};

fn muster(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_muster"))
        .args(args)
        .env_remove("MUSTER_CONFIG_DIR")
        .output()
        .expect("run muster")
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn set_readonly(dir: &Path, ro: bool) {
    for entry in walk(dir) {
        let mode = if entry.is_dir() {
            if ro {
                0o555
            } else {
                0o755
            }
        } else if ro {
            0o444
        } else {
            0o644
        };
        std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(mode)).unwrap();
    }
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![dir.to_path_buf()];
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[test]
fn init_then_validate_read_only() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("cfg");
    let o = muster(&["init", "--out", cfg.to_str().unwrap()]);
    assert!(o.status.success(), "{}", text(&o));
    for f in [
        "muster.toml",
        "workflows/feature.toml",
        "workflows/bugfix.toml",
        "workflows/investigate.toml",
        "prompts/planner.md",
    ] {
        assert!(cfg.join(f).is_file(), "missing {f}");
    }

    set_readonly(&cfg, true);
    let o = muster(&["validate", "--config-dir", cfg.to_str().unwrap()]);
    let out = text(&o);
    set_readonly(&cfg, false);
    assert!(o.status.success(), "{out}");
    assert!(out.contains("3 workflow(s) valid"), "{out}");

    let o = muster(&[
        "--json",
        "list-workflows",
        "--config-dir",
        cfg.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{}", text(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let names: Vec<&str> = v["workflows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["bugfix", "feature", "investigate"]);
}

#[test]
fn init_refuses_non_empty_dir_without_force() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("existing"), "x").unwrap();
    let o = muster(&["init", "--out", tmp.path().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    let o = muster(&["init", "--out", tmp.path().to_str().unwrap(), "--force"]);
    assert!(o.status.success(), "{}", text(&o));
}

#[test]
fn init_refuses_live_config_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_muster"))
        .args(["init", "--force", "--out", tmp.path().to_str().unwrap()])
        .env("MUSTER_CONFIG_DIR", tmp.path())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("live config dir"));
}

#[test]
fn validate_reports_errors_with_exit_2() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path();
    assert!(muster(&["init", "--out", cfg.to_str().unwrap()])
        .status
        .success());
    std::fs::remove_file(cfg.join("prompts/reviewer.md")).unwrap();
    let o = muster(&["validate", "--config-dir", cfg.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        text(&o).contains("prompts/reviewer.md not found"),
        "{}",
        text(&o)
    );

    // Only the named workflow is checked.
    let o = muster(&[
        "validate",
        "investigate",
        "--config-dir",
        cfg.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{}", text(&o));
}

#[test]
fn validate_configmap_layout() {
    // Build the defaults, then rearrange them the way a ConfigMap mount looks:
    // every file is a symlink into ..data, which points at a timestamped dir.
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    assert!(muster(&["init", "--out", src.to_str().unwrap()])
        .status
        .success());

    let cfg = tmp.path().join("cfg");
    let data = cfg.join("..2026_11_25_10_00_00.000000001");
    std::fs::create_dir_all(&data).unwrap();
    for sub in ["workflows", "prompts"] {
        std::fs::create_dir_all(cfg.join(sub)).unwrap();
        std::fs::create_dir_all(data.join(sub)).unwrap();
    }
    for f in walk(&src).into_iter().filter(|p| p.is_file()) {
        let rel = f.strip_prefix(&src).unwrap();
        std::fs::copy(&f, data.join(rel)).unwrap();
        let depth = rel.components().count() - 1;
        let target = format!("{}..data/{}", "../".repeat(depth), rel.display());
        symlink(target, cfg.join(rel)).unwrap();
    }
    symlink("..2026_11_25_10_00_00.000000001", cfg.join("..data")).unwrap();
    // A stale data dir with a broken workflow must be ignored.
    let stale = cfg.join("..2026_11_24_09_00_00.000000001/workflows");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("broken.toml"), "not toml [").unwrap();
    std::fs::write(cfg.join("workflows/.hidden.toml"), "not toml [").unwrap();

    let o = muster(&["validate", "--config-dir", cfg.to_str().unwrap()]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("3 workflow(s) valid"));
}

#[test]
fn defaults_render_without_placeholders() {
    // Every default prompt renders with sample values (validate does this),
    // and the result has no unresolved `{{`.
    let tmp = tempfile::tempdir().unwrap();
    assert!(muster(&["init", "--out", tmp.path().to_str().unwrap()])
        .status
        .success());
    for f in walk(&tmp.path().join("prompts"))
        .into_iter()
        .filter(|p| p.is_file())
    {
        let t = std::fs::read_to_string(&f).unwrap();
        let mut rendered = t.clone();
        for v in [
            "run_id",
            "run_dir",
            "brief_path",
            "produces",
            "iteration",
            "previous_review",
            "repo",
            "branch",
            "base_branch",
            "stage_id",
        ] {
            rendered = rendered.replace(&format!("{{{{{v}}}}}"), "X");
        }
        assert!(
            !rendered.contains("{{"),
            "{} has unresolved placeholders",
            f.display()
        );
        assert!(
            t.contains("{{produces}}"),
            "{} must name its output",
            f.display()
        );
        assert!(
            t.contains("## Blocked"),
            "{} must state the blocked contract",
            f.display()
        );
    }
}
