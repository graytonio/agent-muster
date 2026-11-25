//! `muster doctor`: check every item of the runtime contract (docs/RUNTIME.md).

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use crate::config::Config;
use crate::git::{which, Git};
use crate::herdr::{CliHerdr, Herdr};
use crate::workflow;

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub config_dir: String,
    pub checks: Vec<Check>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }

    pub fn print(&self) {
        for c in &self.checks {
            println!(
                "{} {:<28} {}",
                if c.ok { "ok  " } else { "FAIL" },
                c.name,
                c.detail
            );
            if let (false, Some(fix)) = (c.ok, &c.fix) {
                println!("     {:<28} fix: {fix}", "");
            }
        }
        let failed = self.checks.iter().filter(|c| !c.ok).count();
        if failed == 0 {
            println!("\nall {} checks passed", self.checks.len());
        } else {
            println!("\n{failed} of {} checks failed", self.checks.len());
        }
    }
}

struct Checks(Vec<Check>);

impl Checks {
    fn add(&mut self, name: &str, ok: bool, detail: impl Into<String>, fix: &str) {
        self.0.push(Check {
            name: name.into(),
            ok,
            detail: detail.into(),
            fix: (!ok && !fix.is_empty()).then(|| fix.to_string()),
        });
    }
}

fn writable(dir: &Path) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("{} does not exist", dir.display()));
    }
    let probe = dir.join(format!(".muster-doctor-{}", std::process::id()));
    std::fs::write(&probe, b"x").map_err(|e| format!("{} is not writable: {e}", dir.display()))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

/// The CLI binary for an agent kind, where it differs from the kind name.
fn agent_binary(kind: &str) -> &str {
    match kind {
        "copilot" => "copilot",
        "cursor" => "cursor-agent",
        other => other,
    }
}

pub fn run(config_dir: &Path, verbose: bool) -> Report {
    let mut c = Checks(Vec::new());

    // Config dir and its contents.
    c.add(
        "config dir",
        config_dir.is_dir(),
        config_dir.display().to_string(),
        "mount the config ConfigMap there, or set MUSTER_CONFIG_DIR / --config-dir",
    );
    let cfg = match Config::load(config_dir) {
        Ok(cfg) => {
            c.add("muster.toml", true, "loaded", "");
            Some(cfg)
        }
        Err(e) => {
            c.add(
                "muster.toml",
                false,
                format!("{e:#}"),
                "fix muster.toml (see `muster init`)",
            );
            None
        }
    };

    let mut kinds = BTreeSet::new();
    match workflow::list(config_dir) {
        Ok(names) if !names.is_empty() => {
            for name in names {
                match workflow::load(config_dir, &name) {
                    Ok(l) => {
                        kinds.extend(l.workflow.stages.iter().map(|s| s.kind.clone()));
                        c.add(&format!("workflow {name}"), true, "valid", "");
                    }
                    Err(e) => c.add(
                        &format!("workflow {name}"),
                        false,
                        format!("{e:#}"),
                        "run `muster validate` and fix the reported errors",
                    ),
                }
            }
        }
        Ok(_) => c.add(
            "workflows",
            false,
            "no workflows found",
            "add workflows/*.toml",
        ),
        Err(e) => c.add("workflows", false, format!("{e:#}"), "add workflows/*.toml"),
    }

    let Some(cfg) = cfg else {
        return Report {
            config_dir: config_dir.display().to_string(),
            checks: c.0,
        };
    };

    // herdr.
    match which(&cfg.herdr_bin) {
        Some(p) => {
            c.add("herdr binary", true, p.display().to_string(), "");
            let h = CliHerdr::new(cfg.herdr_bin.clone(), cfg.herdr_session.clone(), verbose);
            match h.status() {
                Ok(v) => {
                    let running = v["running"].as_bool() == Some(true);
                    let compatible = v["compatible"].as_bool() != Some(false);
                    c.add(
                        "herdr server",
                        running && compatible,
                        format!(
                            "{} (version {}, session {})",
                            v["status"].as_str().unwrap_or("?"),
                            v["version"].as_str().unwrap_or("?"),
                            cfg.herdr_session.as_deref().unwrap_or("default")
                        ),
                        "start the herdr server for this session",
                    );
                }
                Err(e) => c.add(
                    "herdr server",
                    false,
                    e.to_string(),
                    "start the herdr server for this session",
                ),
            }
            match h.integration_status() {
                Ok(text) => {
                    for kind in &kinds {
                        let line = text
                            .lines()
                            .find(|l| l.starts_with(&format!("{kind}:")))
                            .unwrap_or("");
                        let ok = line.contains("current");
                        c.add(
                            &format!("herdr integration {kind}"),
                            ok,
                            if line.is_empty() {
                                "not listed".into()
                            } else {
                                line.to_string()
                            },
                            &format!("herdr integration install {kind}"),
                        );
                    }
                }
                Err(e) => c.add("herdr integrations", false, e.to_string(), ""),
            }
        }
        None => c.add(
            "herdr binary",
            false,
            format!("{} not found", cfg.herdr_bin),
            "install herdr, put it on PATH, or set HERDR_BIN",
        ),
    }

    // Agent CLIs.
    for kind in &kinds {
        let bin = agent_binary(kind);
        let found = which(bin);
        c.add(
            &format!("agent cli {bin}"),
            found.is_some(),
            found
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| format!("{bin} not on PATH")),
            &format!("install {bin} and authenticate it"),
        );
    }

    // git, gh, entire.
    let git_found = which("git");
    c.add(
        "git",
        git_found.is_some(),
        git_found
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not on PATH".into()),
        "install git",
    );
    match which(&cfg.gh_bin) {
        Some(p) => {
            c.add("gh", true, p.display().to_string(), "");
            let g = Git::new(cfg.gh_bin.clone(), verbose);
            let auth = g.gh_authenticated(Path::new("/"));
            c.add(
                "gh auth",
                auth.is_ok(),
                auth.err()
                    .map(|e| format!("{e:#}"))
                    .unwrap_or_else(|| "authenticated".into()),
                "set GH_TOKEN or run `gh auth login`",
            );
        }
        None => c.add(
            "gh",
            false,
            format!("{} not on PATH", cfg.gh_bin),
            "install gh or set GH_BIN",
        ),
    }
    if cfg.entire.enabled {
        let g = Git::new(cfg.gh_bin.clone(), verbose);
        let found = which(&g.entire_bin);
        c.add(
            "entire",
            found.is_some(),
            found
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "enabled in muster.toml but not on PATH".into()),
            "install entire, or set [entire] enabled = false",
        );
    }

    // Data dirs.
    let mut dirs = vec![
        ("state_dir", cfg.state_dir.clone()),
        ("log_dir", cfg.log_dir.clone()),
        ("repos_dir", cfg.repos_dir.clone()),
    ];
    if let Some(w) = &cfg.worktrees_dir {
        dirs.push(("worktrees_dir", w.clone()));
    }
    for (name, dir) in dirs {
        let r = writable(&dir);
        c.add(
            name,
            r.is_ok(),
            r.err()
                .unwrap_or_else(|| format!("{} (writable)", dir.display())),
            "create it on the persistent volume, owned by the muster user",
        );
    }

    // Configured repos are cloned.
    for repo in cfg.repos.keys() {
        let p = cfg.repo_path(repo);
        let ok = p.join(".git").exists();
        c.add(
            &format!("repo {repo}"),
            ok,
            if ok {
                p.display().to_string()
            } else {
                format!("not cloned at {}", p.display())
            },
            &format!("git clone <url> {}", p.display()),
        );
    }

    Report {
        config_dir: config_dir.display().to_string(),
        checks: c.0,
    }
}
