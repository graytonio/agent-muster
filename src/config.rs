//! Global config (`muster.toml`) and config-directory access.
//!
//! The config dir is read-only and is usually a mounted Kubernetes ConfigMap:
//! files are symlinks into a hidden `..data` directory that Kubernetes swaps
//! atomically. Listings follow symlinks and skip every entry starting with `.`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::error::MusterError;

pub const DEFAULT_CONFIG_DIR: &str = "/etc/muster";
pub const CONFIG_FILE: &str = "muster.toml";
pub const WORKFLOWS_DIR: &str = "workflows";
pub const PROMPTS_DIR: &str = "prompts";

/// Resolve the config dir: `--config-dir`, then `$MUSTER_CONFIG_DIR`, then `/etc/muster`.
pub fn resolve_config_dir(flag: Option<&Path>, env: Option<OsString>) -> PathBuf {
    if let Some(p) = flag {
        return p.to_path_buf();
    }
    match env {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(DEFAULT_CONFIG_DIR),
    }
}

/// Expand a leading `~` or `~/` using `home`.
pub fn expand_tilde(s: &str, home: Option<&Path>) -> PathBuf {
    match (s, home) {
        ("~", Some(h)) => h.to_path_buf(),
        (s, Some(h)) if s.starts_with("~/") => h.join(&s[2..]),
        (s, _) => PathBuf::from(s),
    }
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// List regular files (following symlinks) in `dir` with the given extension,
/// skipping hidden entries such as ConfigMap's `..data` and `..2026_...` dirs.
/// Returns file stems, sorted.
pub fn list_stems(dir: &Path, ext: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        // fs::metadata follows symlinks; a dangling link is skipped.
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                out.push(stem.to_string());
            }
        }
    }
    out.sort();
    Ok(out)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawConfig {
    pub repos_dir: String,
    pub state_dir: String,
    pub log_dir: String,
    /// Where run worktrees go. Empty: herdr's default location.
    #[serde(default)]
    pub worktrees_dir: String,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_runs: u32,
    #[serde(default)]
    pub herdr: HerdrConfig,
    #[serde(default)]
    pub git: GitConfig,
    #[serde(default)]
    pub entire: EntireConfig,
    #[serde(default)]
    pub repos: BTreeMap<String, RepoConfig>,
}

fn default_max_concurrent() -> u32 {
    2
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HerdrConfig {
    #[serde(default = "default_herdr_bin")]
    pub bin: String,
    /// Empty: the default session. Otherwise passed as `--session`.
    #[serde(default)]
    pub session: String,
}

fn default_herdr_bin() -> String {
    "herdr".into()
}

impl Default for HerdrConfig {
    fn default() -> Self {
        Self {
            bin: default_herdr_bin(),
            session: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GitConfig {
    #[serde(default = "default_remote")]
    pub remote: String,
    #[serde(default = "default_base_branch")]
    pub base_branch: String,
}

fn default_remote() -> String {
    "origin".into()
}
fn default_base_branch() -> String {
    "main".into()
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            remote: default_remote(),
            base_branch: default_base_branch(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EntireConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Agents passed to `entire enable --agent`, one call each.
    #[serde(default = "default_entire_agents")]
    pub agents: Vec<String>,
    /// `PROVIDER:OWNER/REPO`, for example `github:me/muster-checkpoints`.
    #[serde(default)]
    pub checkpoint_remote: String,
}

fn default_entire_agents() -> Vec<String> {
    vec!["claude-code".into()]
}

impl Default for EntireConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            agents: default_entire_agents(),
            checkpoint_remote: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    #[serde(default)]
    pub base_branch: Option<String>,
}

/// Loaded global config with paths expanded.
#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub config_dir: PathBuf,
    pub repos_dir: PathBuf,
    pub state_dir: PathBuf,
    pub log_dir: PathBuf,
    pub worktrees_dir: Option<PathBuf>,
    pub max_concurrent_runs: u32,
    pub herdr_bin: String,
    pub herdr_session: Option<String>,
    pub gh_bin: String,
    pub git: GitConfig,
    pub entire: EntireConfig,
    pub repos: BTreeMap<String, RepoConfig>,
}

impl Config {
    /// Load `<dir>/muster.toml`, applying `~` expansion and env overrides
    /// (`HERDR_BIN`, `GH_BIN`).
    pub fn load(dir: &Path) -> Result<Config> {
        let path = dir.join(CONFIG_FILE);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| MusterError::config(format!("cannot read {}: {e}", path.display())))?;
        let raw: RawConfig = toml::from_str(&text)
            .map_err(|e| MusterError::config(format!("{}: {e}", path.display())))?;
        Self::from_raw(dir, raw, home_dir().as_deref(), &EnvOverrides::from_env())
    }

    pub fn from_raw(
        dir: &Path,
        raw: RawConfig,
        home: Option<&Path>,
        env: &EnvOverrides,
    ) -> Result<Config> {
        if raw.max_concurrent_runs == 0 {
            bail!(MusterError::config(
                "max_concurrent_runs must be at least 1"
            ));
        }
        if raw.entire.enabled && raw.entire.agents.is_empty() {
            bail!(MusterError::config(
                "entire.agents must name at least one agent when entire is enabled"
            ));
        }
        let worktrees_dir = if raw.worktrees_dir.is_empty() {
            None
        } else {
            Some(expand_tilde(&raw.worktrees_dir, home))
        };
        Ok(Config {
            config_dir: dir.to_path_buf(),
            repos_dir: expand_tilde(&raw.repos_dir, home),
            state_dir: expand_tilde(&raw.state_dir, home),
            log_dir: expand_tilde(&raw.log_dir, home),
            worktrees_dir,
            max_concurrent_runs: raw.max_concurrent_runs,
            herdr_bin: env
                .herdr_bin
                .clone()
                .unwrap_or_else(|| raw.herdr.bin.clone()),
            herdr_session: Some(raw.herdr.session.clone()).filter(|s| !s.is_empty()),
            gh_bin: env.gh_bin.clone().unwrap_or_else(|| "gh".into()),
            git: raw.git,
            entire: raw.entire,
            repos: raw.repos,
        })
    }

    pub fn base_branch_for(&self, repo: &str) -> String {
        self.repos
            .get(repo)
            .and_then(|r| r.base_branch.clone())
            .unwrap_or_else(|| self.git.base_branch.clone())
    }

    pub fn repo_path(&self, repo: &str) -> PathBuf {
        self.repos_dir.join(repo)
    }

    pub fn workflows_dir(&self) -> PathBuf {
        self.config_dir.join(WORKFLOWS_DIR)
    }

    pub fn prompts_dir(&self) -> PathBuf {
        self.config_dir.join(PROMPTS_DIR)
    }
}

#[derive(Debug, Clone, Default)]
pub struct EnvOverrides {
    pub herdr_bin: Option<String>,
    pub gh_bin: Option<String>,
}

impl EnvOverrides {
    pub fn from_env() -> Self {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Self {
            herdr_bin: get("HERDR_BIN"),
            gh_bin: get("GH_BIN"),
        }
    }
}

/// Valid repo name: a single path component under `repos_dir`.
pub fn check_repo_name(repo: &str) -> Result<()> {
    let ok = !repo.is_empty()
        && repo != "."
        && repo != ".."
        && !repo.contains('/')
        && !repo.contains('\\')
        && !repo.starts_with('.');
    if !ok {
        bail!(MusterError::usage(format!(
            "invalid repo name {repo:?}: must be a directory name under repos_dir"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn config_dir_lookup_order() {
        let flag = PathBuf::from("/flag");
        assert_eq!(
            resolve_config_dir(Some(&flag), Some("/env".into())),
            PathBuf::from("/flag")
        );
        assert_eq!(
            resolve_config_dir(None, Some("/env".into())),
            PathBuf::from("/env")
        );
        assert_eq!(
            resolve_config_dir(None, Some("".into())),
            PathBuf::from(DEFAULT_CONFIG_DIR)
        );
        assert_eq!(
            resolve_config_dir(None, None),
            PathBuf::from(DEFAULT_CONFIG_DIR)
        );
    }

    #[test]
    fn tilde_expansion() {
        let home = Path::new("/home/m");
        assert_eq!(expand_tilde("~", Some(home)), PathBuf::from("/home/m"));
        assert_eq!(
            expand_tilde("~/repos", Some(home)),
            PathBuf::from("/home/m/repos")
        );
        assert_eq!(expand_tilde("/abs", Some(home)), PathBuf::from("/abs"));
        assert_eq!(
            expand_tilde("~other/x", Some(home)),
            PathBuf::from("~other/x")
        );
        assert_eq!(expand_tilde("~/x", None), PathBuf::from("~/x"));
    }

    fn raw(text: &str) -> RawConfig {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn loads_with_defaults_and_overrides() {
        let r = raw(r#"
            repos_dir = "~/repos"
            state_dir = "~/.muster/state"
            log_dir = "/var/log/muster"
            [herdr]
            session = "work"
            [repos.foo]
            base_branch = "develop"
        "#);
        let env = EnvOverrides {
            herdr_bin: Some("/opt/herdr".into()),
            gh_bin: None,
        };
        let c = Config::from_raw(Path::new("/cfg"), r, Some(Path::new("/h")), &env).unwrap();
        assert_eq!(c.repos_dir, PathBuf::from("/h/repos"));
        assert_eq!(c.state_dir, PathBuf::from("/h/.muster/state"));
        assert_eq!(c.log_dir, PathBuf::from("/var/log/muster"));
        assert_eq!(c.max_concurrent_runs, 2);
        assert_eq!(c.herdr_bin, "/opt/herdr");
        assert_eq!(c.herdr_session.as_deref(), Some("work"));
        assert_eq!(c.gh_bin, "gh");
        assert_eq!(c.base_branch_for("foo"), "develop");
        assert_eq!(c.base_branch_for("bar"), "main");
        assert!(c.worktrees_dir.is_none());
        assert!(!c.entire.enabled);
    }

    #[test]
    fn rejects_unknown_keys_and_zero_concurrency() {
        let bad: Result<RawConfig, _> =
            toml::from_str("repos_dir='a'\nstate_dir='b'\nlog_dir='c'\nbogus=1");
        assert!(bad.is_err());
        let r = raw("repos_dir='a'\nstate_dir='b'\nlog_dir='c'\nmax_concurrent_runs=0");
        assert!(Config::from_raw(Path::new("/"), r, None, &EnvOverrides::default()).is_err());
    }

    #[test]
    fn listing_follows_symlinks_and_skips_dot_entries() {
        // Mimic a Kubernetes ConfigMap mount:
        //   ..2026_11_25_10_00_00.123/feature.toml   (current data)
        //   ..2026_11_24_09_00_00.999/old.toml       (stale data dir)
        //   ..data -> ..2026_11_25_10_00_00.123
        //   feature.toml -> ..data/feature.toml
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        let current = d.join("..2026_11_25_10_00_00.123");
        let stale = d.join("..2026_11_24_09_00_00.999");
        std::fs::create_dir(&current).unwrap();
        std::fs::create_dir(&stale).unwrap();
        std::fs::write(current.join("feature.toml"), "x").unwrap();
        std::fs::write(stale.join("old.toml"), "x").unwrap();
        symlink("..2026_11_25_10_00_00.123", d.join("..data")).unwrap();
        symlink("..data/feature.toml", d.join("feature.toml")).unwrap();
        symlink("..data/missing.toml", d.join("dangling.toml")).unwrap();
        std::fs::write(d.join(".hidden.toml"), "x").unwrap();
        std::fs::write(d.join("notes.txt"), "x").unwrap();

        assert_eq!(list_stems(d, "toml").unwrap(), vec!["feature".to_string()]);
    }

    #[test]
    fn repo_names() {
        assert!(check_repo_name("muster-sandbox").is_ok());
        for bad in ["", ".", "..", "a/b", ".git"] {
            assert!(check_repo_name(bad).is_err(), "{bad}");
        }
    }
}
