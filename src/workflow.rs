//! Workflow TOML schema, loading and validation.
//!
//! A workflow and the prompts it uses are loaded from a directory laid out like
//! the config dir (`workflows/<name>.toml`, `prompts/<role>.md`). The same loader
//! reads the live config dir and a run's config snapshot.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::config::{list_stems, PROMPTS_DIR, WORKFLOWS_DIR};
use crate::error::MusterError;
use crate::render;

/// Agent kinds supported by herdr 0.9.3 (`herdr agent` usage line).
pub const AGENT_KINDS: &[&str] = &[
    "pi",
    "claude",
    "codex",
    "gemini",
    "cursor",
    "devin",
    "agy",
    "cline",
    "omp",
    "mastracode",
    "opencode",
    "copilot",
    "kimi",
    "kiro",
    "droid",
    "amp",
    "grok",
    "hermes",
    "kilo",
    "qodercli",
    "qwen",
    "letta",
    "maki",
    "muse",
];

/// herdr agent names are `[a-z][a-z0-9_-]{0,31}`. Agent names are
/// `<role>-<run_id>`, so roles and run ids share 31 characters plus the dash.
pub const MAX_ROLE_LEN: usize = 20;
pub const MAX_RUN_ID_LEN: usize = 11;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub workflow: WorkflowMeta,
    #[serde(rename = "stage", default)]
    pub stages: Vec<Stage>,
    #[serde(default)]
    pub finish: Finish,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowMeta {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub branch_prefix: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    pub id: String,
    pub role: String,
    pub kind: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Artifact path, relative to the run dir.
    pub produces: String,
    #[serde(default = "default_timeout")]
    pub timeout_min: u64,
    /// Line 1 of the artifact must be `VERDICT: APPROVE` or `VERDICT: CHANGES`.
    #[serde(default)]
    pub verdict: bool,
    /// Stage to loop back to on `VERDICT: CHANGES`.
    #[serde(default)]
    pub on_changes: Option<String>,
    #[serde(default)]
    pub max_iterations: Option<u32>,
    /// Can be skipped with `--skip <id>`.
    #[serde(default)]
    pub optional: bool,
    /// The stage must not change files outside the run dir. Enforced by the
    /// conductor after the stage settles.
    #[serde(default)]
    pub read_only: bool,
}

fn default_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Finish {
    #[serde(default = "yes")]
    pub open_pr: bool,
    #[serde(default)]
    pub draft: bool,
    /// Artifacts (relative to the run dir) concatenated into the PR body.
    #[serde(default)]
    pub pr_body_from: Vec<String>,
}

fn yes() -> bool {
    true
}

impl Default for Finish {
    fn default() -> Self {
        Self {
            open_pr: true,
            draft: false,
            pr_body_from: Vec::new(),
        }
    }
}

impl Workflow {
    pub fn parse(text: &str) -> Result<Workflow, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn stage_index(&self, id: &str) -> Option<usize> {
        self.stages.iter().position(|s| s.id == id)
    }

    pub fn roles(&self) -> BTreeSet<&str> {
        self.stages.iter().map(|s| s.role.as_str()).collect()
    }

    /// Validate the workflow. `expected_name` is the file stem; `prompt_exists`
    /// says whether `prompts/<role>.md` exists. Returns every problem found.
    pub fn validate(
        &self,
        expected_name: &str,
        prompt_exists: &dyn Fn(&str) -> bool,
    ) -> Vec<String> {
        let mut errs = Vec::new();
        let w = &self.workflow;
        if w.name != expected_name {
            errs.push(format!(
                "workflow.name {:?} must match the file name {:?}",
                w.name, expected_name
            ));
        }
        if !is_ident(&w.name) {
            errs.push(format!(
                "workflow.name {:?} must match [a-z][a-z0-9_-]*",
                w.name
            ));
        }
        if !valid_branch_prefix(&w.branch_prefix) {
            errs.push(format!(
                "workflow.branch_prefix {:?} must be non-empty, without spaces, `..`, or a leading/trailing `/`",
                w.branch_prefix
            ));
        }
        if self.stages.is_empty() {
            errs.push("workflow has no [[stage]] entries".into());
        }

        let mut ids = BTreeSet::new();
        let mut produced = BTreeMap::new();
        for (i, s) in self.stages.iter().enumerate() {
            let at = format!("stage {:?}", s.id);
            if !ids.insert(s.id.as_str()) {
                errs.push(format!("{at}: duplicate stage id"));
            }
            if !is_ident(&s.id) {
                errs.push(format!("{at}: id must match [a-z][a-z0-9_-]*"));
            }
            if !is_ident(&s.role) || s.role.len() > MAX_ROLE_LEN {
                errs.push(format!(
                    "{at}: role {:?} must match [a-z][a-z0-9_-]* and be at most {MAX_ROLE_LEN} characters",
                    s.role
                ));
            } else if !prompt_exists(&s.role) {
                errs.push(format!(
                    "{at}: prompt file {PROMPTS_DIR}/{}.md not found",
                    s.role
                ));
            }
            if !AGENT_KINDS.contains(&s.kind.as_str()) {
                errs.push(format!(
                    "{at}: kind {:?} is not a herdr agent kind ({})",
                    s.kind,
                    AGENT_KINDS.join("|")
                ));
            }
            if !is_plain_relative(&s.produces) {
                errs.push(format!(
                    "{at}: produces {:?} must be a plain relative path inside the run dir",
                    s.produces
                ));
            } else if let Some(prev) = produced.insert(s.produces.as_str(), s.id.as_str()) {
                errs.push(format!(
                    "{at}: produces {:?} is already produced by stage {prev:?}",
                    s.produces
                ));
            } else if is_reserved_artifact(&s.produces) {
                errs.push(format!(
                    "{at}: produces {:?} collides with a file muster writes (brief.md, prompts/, config/)",
                    s.produces
                ));
            }
            if s.timeout_min == 0 {
                errs.push(format!("{at}: timeout_min must be at least 1"));
            }
            if s.verdict {
                match s.max_iterations {
                    Some(n) if n >= 1 => {}
                    _ => errs.push(format!("{at}: verdict stages need max_iterations >= 1")),
                }
                match &s.on_changes {
                    None => errs.push(format!(
                        "{at}: verdict stages need on_changes (the stage to loop back to)"
                    )),
                    Some(target) => match self.stage_index(target) {
                        Some(j) if j < i => {
                            if self.stages[j].optional {
                                errs.push(format!(
                                    "{at}: on_changes target {target:?} must not be optional"
                                ));
                            }
                        }
                        Some(_) => errs.push(format!(
                            "{at}: on_changes {target:?} must refer to an earlier stage"
                        )),
                        None => errs.push(format!("{at}: on_changes {target:?} is not a stage id")),
                    },
                }
            } else {
                if s.on_changes.is_some() {
                    errs.push(format!("{at}: on_changes requires verdict = true"));
                }
                if s.max_iterations.is_some() {
                    errs.push(format!("{at}: max_iterations requires verdict = true"));
                }
            }
        }

        for f in &self.finish.pr_body_from {
            if !produced.contains_key(f.as_str()) {
                errs.push(format!(
                    "finish.pr_body_from: {f:?} is not produced by any stage"
                ));
            }
        }
        errs
    }

    /// Resolve `--skip` values: each must name an optional stage.
    pub fn resolve_skips(&self, skip: &[String]) -> Result<BTreeSet<String>> {
        let mut out = BTreeSet::new();
        for id in skip {
            match self.stages.iter().find(|s| &s.id == id) {
                None => bail!(MusterError::usage(format!(
                    "--skip {id}: no such stage in workflow {}",
                    self.workflow.name
                ))),
                Some(s) if !s.optional => bail!(MusterError::usage(format!(
                    "--skip {id}: stage is not optional"
                ))),
                Some(_) => {
                    out.insert(id.clone());
                }
            }
        }
        Ok(out)
    }
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn valid_branch_prefix(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('/')
        && !s.ends_with('/')
        && !s.contains("..")
        && !s.contains("//")
        && s.chars()
            .all(|c| c.is_ascii_graphic() && !"~^:?*[\\".contains(c))
}

/// A non-empty relative path made only of normal components.
pub fn is_plain_relative(p: &str) -> bool {
    !p.is_empty()
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn is_reserved_artifact(p: &str) -> bool {
    let first = Path::new(p)
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .unwrap_or("");
    p == "brief.md" || first == "prompts" || first == "config"
}

/// A workflow loaded from a config-style directory, with its prompts.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub name: String,
    pub workflow: Workflow,
    /// Raw TOML, kept verbatim for the snapshot.
    pub source: String,
    /// role -> template text
    pub prompts: BTreeMap<String, String>,
}

/// Names of the workflows in `<dir>/workflows/`.
pub fn list(dir: &Path) -> Result<Vec<String>> {
    let wdir = dir.join(WORKFLOWS_DIR);
    if !wdir.is_dir() {
        bail!(MusterError::config(format!(
            "{} does not exist",
            wdir.display()
        )));
    }
    list_stems(&wdir, "toml")
}

/// Load and fully validate workflow `name` from `dir` (a config dir or a
/// snapshot). Validation includes rendering every prompt with sample values.
pub fn load(dir: &Path, name: &str) -> Result<Loaded> {
    if !is_ident(name) {
        bail!(MusterError::usage(format!(
            "invalid workflow name {name:?}"
        )));
    }
    let path = dir.join(WORKFLOWS_DIR).join(format!("{name}.toml"));
    let source = std::fs::read_to_string(&path).map_err(|e| {
        MusterError::config(format!(
            "workflow {name:?}: cannot read {}: {e}",
            path.display()
        ))
    })?;
    let workflow = Workflow::parse(&source)
        .map_err(|e| MusterError::config(format!("{}: {e}", path.display())))?;

    let pdir = dir.join(PROMPTS_DIR);
    let prompt_path = |role: &str| pdir.join(format!("{role}.md"));
    let mut errs = workflow.validate(name, &|role| prompt_path(role).is_file());

    let mut prompts = BTreeMap::new();
    for role in workflow.roles() {
        let p = prompt_path(role);
        if !p.is_file() {
            continue; // already reported
        }
        match std::fs::read_to_string(&p) {
            Ok(text) => {
                if let Err(e) = render::render(&text, &render::sample_vars()) {
                    errs.push(format!("{PROMPTS_DIR}/{role}.md: {e}"));
                }
                prompts.insert(role.to_string(), text);
            }
            Err(e) => errs.push(format!("{}: {e}", p.display())),
        }
    }

    if !errs.is_empty() {
        let mut msg = format!("workflow {name:?} is invalid ({}):", path.display());
        for e in &errs {
            msg.push_str("\n  - ");
            msg.push_str(e);
        }
        bail!(MusterError::config(msg));
    }
    Ok(Loaded {
        name: name.to_string(),
        workflow,
        source,
        prompts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
[workflow]
name = "feature"
branch_prefix = "muster/feat"

[[stage]]
id = "plan"
role = "planner"
kind = "claude"
produces = "plan.md"

[[stage]]
id = "build"
role = "builder"
kind = "claude"
produces = "build-report.md"

[[stage]]
id = "review"
role = "reviewer"
kind = "claude"
produces = "review.md"
verdict = true
on_changes = "build"
max_iterations = 2

[[stage]]
id = "tutorial"
role = "tutorial"
kind = "claude"
produces = "tutorial.md"
optional = true

[finish]
pr_body_from = ["plan.md", "review.md"]
"#;

    fn errors(text: &str) -> Vec<String> {
        Workflow::parse(text)
            .unwrap()
            .validate("feature", &|_| true)
    }

    fn mutate(from: &str, to: &str) -> Vec<String> {
        assert!(GOOD.contains(from), "fixture missing {from}");
        errors(&GOOD.replacen(from, to, 1))
    }

    fn has(errs: &[String], needle: &str) -> bool {
        errs.iter().any(|e| e.contains(needle))
    }

    #[test]
    fn good_workflow_validates() {
        assert_eq!(errors(GOOD), Vec::<String>::new());
        let w = Workflow::parse(GOOD).unwrap();
        assert_eq!(w.stages[0].timeout_min, 30);
        assert!(w.finish.open_pr);
        assert!(!w.finish.draft);
    }

    #[test]
    fn name_must_match_file() {
        assert!(has(
            &mutate("name = \"feature\"", "name = \"other\""),
            "must match the file name"
        ));
    }

    #[test]
    fn duplicate_ids() {
        assert!(has(
            &mutate("id = \"build\"", "id = \"plan\""),
            "duplicate stage id"
        ));
    }

    #[test]
    fn on_changes_must_be_earlier_and_exist() {
        assert!(has(
            &mutate("on_changes = \"build\"", "on_changes = \"tutorial\""),
            "earlier stage"
        ));
        assert!(has(
            &mutate("on_changes = \"build\"", "on_changes = \"nope\""),
            "not a stage id"
        ));
        assert!(has(
            &mutate("on_changes = \"build\"", "on_changes = \"review\""),
            "earlier stage"
        ));
    }

    #[test]
    fn verdict_needs_iterations_and_target() {
        assert!(has(
            &mutate("max_iterations = 2", "max_iterations = 0"),
            "max_iterations >= 1"
        ));
        assert!(has(
            &mutate("max_iterations = 2", ""),
            "max_iterations >= 1"
        ));
        assert!(has(
            &mutate("on_changes = \"build\"", ""),
            "need on_changes"
        ));
        assert!(has(
            &mutate("verdict = true", "verdict = false"),
            "on_changes requires verdict"
        ));
    }

    #[test]
    fn missing_prompt() {
        let w = Workflow::parse(GOOD).unwrap();
        let errs = w.validate("feature", &|role| role != "builder");
        assert!(has(&errs, "prompts/builder.md not found"));
    }

    #[test]
    fn bad_kind() {
        assert!(has(
            &mutate("kind = \"claude\"", "kind = \"gpt\""),
            "not a herdr agent kind"
        ));
    }

    #[test]
    fn produces_must_be_plain_relative() {
        for bad in ["/abs.md", "../up.md", "a/../b.md", "./x.md", ""] {
            let errs = mutate("produces = \"plan.md\"", &format!("produces = {bad:?}"));
            assert!(has(&errs, "plain relative path"), "{bad}: {errs:?}");
        }
        assert!(has(
            &mutate("produces = \"plan.md\"", "produces = \"brief.md\""),
            "collides"
        ));
        assert!(has(
            &mutate("produces = \"plan.md\"", "produces = \"review.md\""),
            "already produced"
        ));
        assert!(
            mutate("produces = \"plan.md\"", "produces = \"out/plan.md\"")
                .iter()
                .all(|e| !e.contains("plain relative"))
        );
    }

    #[test]
    fn misc_errors() {
        assert!(has(
            &mutate(
                "branch_prefix = \"muster/feat\"",
                "branch_prefix = \"a b/\""
            ),
            "branch_prefix"
        ));
        assert!(has(
            &mutate("role = \"planner\"", "role = \"Planner\""),
            "role"
        ));
        assert!(has(
            &mutate(
                "role = \"planner\"",
                "role = \"a-very-long-role-name-indeed\""
            ),
            "at most"
        ));
        assert!(has(
            &mutate(
                "produces = \"plan.md\"\n",
                "produces = \"plan.md\"\ntimeout_min = 0\n"
            ),
            "timeout_min"
        ));
        assert!(has(
            &mutate("[\"plan.md\", \"review.md\"]", "[\"nope.md\"]"),
            "not produced"
        ));
        assert!(Workflow::parse("[workflow]\nname='x'\nbranch_prefix='p'\nbogus=1").is_err());
    }

    #[test]
    fn skip_only_optional_stages() {
        let w = Workflow::parse(GOOD).unwrap();
        let s = w.resolve_skips(&["tutorial".into()]).unwrap();
        assert!(s.contains("tutorial"));
        assert!(w.resolve_skips(&["build".into()]).is_err());
        assert!(w.resolve_skips(&["nope".into()]).is_err());
        assert!(w.resolve_skips(&[]).unwrap().is_empty());
    }

    #[test]
    fn load_reports_prompt_render_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::fs::create_dir_all(d.join("workflows")).unwrap();
        std::fs::create_dir_all(d.join("prompts")).unwrap();
        std::fs::write(d.join("workflows/feature.toml"), GOOD).unwrap();
        for r in ["planner", "builder", "reviewer", "tutorial"] {
            std::fs::write(d.join(format!("prompts/{r}.md")), "write {{produces}}").unwrap();
        }
        let loaded = load(d, "feature").unwrap();
        assert_eq!(loaded.prompts.len(), 4);

        std::fs::write(d.join("prompts/builder.md"), "oops {{nope}}").unwrap();
        let err = load(d, "feature").unwrap_err().to_string();
        assert!(
            err.contains("prompts/builder.md") && err.contains("unknown variable"),
            "{err}"
        );
    }
}
