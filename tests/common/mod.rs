//! Shared helpers for integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Create `<root>/repos/<name>` with one commit on `main`, pushed to a bare
/// remote at `<root>/remotes/<name>.git` as `origin`.
pub fn make_repo(root: &Path, name: &str) -> PathBuf {
    let repo = root.join("repos").join(name);
    let remote = root.join("remotes").join(format!("{name}.git"));
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "-q", "--bare", "-b", "main"]);
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    std::fs::write(repo.join("README.md"), "# sandbox\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&repo, &["push", "-q", "-u", "origin", "main"]);
    repo
}

pub fn fake_herdr_bin() -> &'static str {
    env!("CARGO_BIN_EXE_fake-herdr")
}

/// Read fake-herdr's invocation log: one argv per call.
pub fn herdr_log(state: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(state.join("log.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
