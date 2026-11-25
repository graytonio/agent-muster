//! Every git, gh and entire call lives here.

use std::path::Path;
use std::process::{Command, Output};

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone)]
pub struct Git {
    pub gh_bin: String,
    pub entire_bin: String,
    pub verbose: bool,
}

impl Git {
    pub fn new(gh_bin: impl Into<String>, verbose: bool) -> Self {
        Self {
            gh_bin: gh_bin.into(),
            entire_bin: std::env::var("ENTIRE_BIN")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "entire".into()),
            verbose,
        }
    }

    fn exec(&self, bin: &str, dir: &Path, args: &[&str]) -> Result<Output> {
        if self.verbose {
            eprintln!("+ (cd {}) {bin} {}", dir.display(), args.join(" "));
        }
        Command::new(bin)
            .current_dir(dir)
            .args(args)
            .output()
            .with_context(|| format!("cannot run {bin}"))
    }

    /// Run and return trimmed stdout; non-zero exit is an error with stderr.
    fn run(&self, bin: &str, dir: &Path, args: &[&str]) -> Result<String> {
        let out = self.exec(bin, dir, args)?;
        if !out.status.success() {
            bail!(
                "`{bin} {}` failed in {}: {}",
                args.join(" "),
                dir.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    pub fn git(&self, dir: &Path, args: &[&str]) -> Result<String> {
        self.run("git", dir, args)
    }

    pub fn fetch(&self, repo: &Path, remote: &str) -> Result<()> {
        self.git(repo, &["fetch", "--quiet", remote]).map(|_| ())
    }

    pub fn head(&self, dir: &Path) -> Result<String> {
        self.git(dir, &["rev-parse", "HEAD"])
    }

    pub fn is_dirty(&self, dir: &Path) -> Result<bool> {
        Ok(!self.git(dir, &["status", "--porcelain"])?.is_empty())
    }

    /// Paths that are modified, staged or untracked right now.
    pub fn dirty_paths(&self, dir: &Path) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        for o in [
            self.git(dir, &["diff", "--name-only", "HEAD"])?,
            self.git(dir, &["ls-files", "--others", "--exclude-standard"])?,
        ] {
            out.extend(o.lines().filter(|l| !l.is_empty()).map(str::to_string));
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Stage and commit everything under `path` (relative to `dir`). Returns
    /// whether a commit was made.
    pub fn commit_path(&self, dir: &Path, path: &Path, message: &str) -> Result<bool> {
        let p = path.to_str().context("non-UTF-8 path")?;
        self.git(dir, &["add", "--all", "--", p])?;
        let staged = self.exec("git", dir, &["diff", "--cached", "--quiet", "--", p])?;
        if staged.status.success() {
            return Ok(false);
        }
        self.git(
            dir,
            &["commit", "--quiet", "--no-verify", "-m", message, "--", p],
        )?;
        Ok(true)
    }

    /// Stage and commit every change in the worktree.
    pub fn commit_all(&self, dir: &Path, message: &str) -> Result<bool> {
        self.git(dir, &["add", "--all"])?;
        let staged = self.exec("git", dir, &["diff", "--cached", "--quiet"])?;
        if staged.status.success() {
            return Ok(false);
        }
        self.git(dir, &["commit", "--quiet", "--no-verify", "-m", message])?;
        Ok(true)
    }

    /// Files changed since `since` (committed or not, plus untracked files)
    /// that are not under `allowed` (relative to `dir`).
    pub fn changed_outside(&self, dir: &Path, since: &str, allowed: &Path) -> Result<Vec<String>> {
        let allowed = allowed.to_str().context("non-UTF-8 path")?;
        let prefix = format!("{}/", allowed.trim_end_matches('/'));
        let mut files: Vec<String> = Vec::new();
        for out in [
            self.git(dir, &["diff", "--name-only", since])?,
            self.git(dir, &["ls-files", "--others", "--exclude-standard"])?,
        ] {
            files.extend(out.lines().map(str::to_string));
        }
        files.retain(|f| !f.is_empty() && !f.starts_with(&prefix));
        files.sort();
        files.dedup();
        Ok(files)
    }

    pub fn push(&self, dir: &Path, remote: &str, branch: &str) -> Result<()> {
        self.git(dir, &["push", "--quiet", "-u", remote, branch])
            .map(|_| ())
    }

    pub fn branch_exists(&self, repo: &Path, branch: &str) -> bool {
        self.exec(
            "git",
            repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
        .map(|o| o.status.success())
        .unwrap_or(false)
    }

    pub fn delete_branch(&self, repo: &Path, branch: &str) -> Result<()> {
        self.git(repo, &["branch", "-D", branch]).map(|_| ())
    }

    /// `gh pr create`, returning the PR URL (last line of stdout).
    pub fn pr_create(
        &self,
        dir: &Path,
        base: &str,
        head: &str,
        title: &str,
        body_file: &Path,
        draft: bool,
    ) -> Result<String> {
        let body = body_file.to_str().context("non-UTF-8 path")?;
        let mut args = vec![
            "pr",
            "create",
            "--base",
            base,
            "--head",
            head,
            "--title",
            title,
            "--body-file",
            body,
        ];
        if draft {
            args.push("--draft");
        }
        let out = self.run(&self.gh_bin, dir, &args)?;
        Ok(out.lines().last().unwrap_or("").trim().to_string())
    }

    /// `gh auth status` succeeds.
    pub fn gh_authenticated(&self, dir: &Path) -> Result<()> {
        self.run(&self.gh_bin, dir, &["auth", "status"]).map(|_| ())
    }

    /// `entire enable --yes --agent <a>`, then the optional checkpoint remote.
    pub fn entire_setup(
        &self,
        dir: &Path,
        agents: &[String],
        checkpoint_remote: &str,
    ) -> Result<()> {
        for a in agents {
            self.run(&self.entire_bin, dir, &["enable", "--yes", "--agent", a])?;
        }
        if !checkpoint_remote.is_empty() {
            self.run(
                &self.entire_bin,
                dir,
                &[
                    "configure",
                    "--local",
                    "--checkpoint-remote",
                    checkpoint_remote,
                ],
            )?;
        }
        Ok(())
    }

    /// `entire checkpoint list --outgoing --json`.
    pub fn entire_outgoing(&self, dir: &Path) -> Result<String> {
        self.run(
            &self.entire_bin,
            dir,
            &["checkpoint", "list", "--outgoing", "--json", "--no-pager"],
        )
    }
}

/// Whether `bin` resolves to an executable (absolute path or on PATH).
pub fn which(bin: &str) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let is_exec = |p: &Path| {
        std::fs::metadata(p)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    };
    if bin.contains('/') {
        let p = Path::new(bin);
        return is_exec(p).then(|| p.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(bin))
            .find(|p| is_exec(p))
    })
}
