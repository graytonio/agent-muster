//! Strict `{{name}}` prompt template rendering.
//!
//! Unknown variables are errors, and so is any `{{` that does not form a
//! complete, known placeholder.

use std::collections::BTreeMap;

use thiserror::Error;

/// Every variable a template may use.
pub const VARIABLES: &[&str] = &[
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
];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RenderError {
    #[error("line {line}: unknown variable {{{{{name}}}}}")]
    UnknownVariable { name: String, line: usize },
    #[error("line {line}: no value provided for {{{{{name}}}}}")]
    MissingValue { name: String, line: usize },
    #[error("line {line}: unclosed `{{{{`")]
    Unclosed { line: usize },
}

pub type Vars = BTreeMap<&'static str, String>;

/// Render `template`, substituting each `{{name}}` (surrounding spaces allowed).
pub fn render(template: &str, vars: &Vars) -> Result<String, RenderError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    let mut consumed = 0usize;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let line = line_of(template, consumed + start);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return Err(RenderError::Unclosed { line });
        };
        let raw = &after[..end];
        let name = raw.trim();
        // A `{{` inside the placeholder means the first one was never closed.
        if raw.contains("{{") || raw.contains('\n') {
            return Err(RenderError::Unclosed { line });
        }
        let Some(known) = VARIABLES.iter().find(|v| **v == name) else {
            return Err(RenderError::UnknownVariable {
                name: name.to_string(),
                line,
            });
        };
        let Some(value) = vars.get(known) else {
            return Err(RenderError::MissingValue {
                name: name.to_string(),
                line,
            });
        };
        out.push_str(value);
        let advance = start + 2 + end + 2;
        consumed += advance;
        rest = &rest[advance..];
    }
    out.push_str(rest);
    Ok(out)
}

fn line_of(s: &str, byte: usize) -> usize {
    s[..byte].matches('\n').count() + 1
}

/// Sample values for validating templates without a run.
pub fn sample_vars() -> Vars {
    let mut v = Vars::new();
    for (k, val) in [
        ("run_id", "f1"),
        ("run_dir", "/work/.muster/runs/f1"),
        ("brief_path", "/work/.muster/runs/f1/brief.md"),
        ("produces", "/work/.muster/runs/f1/plan.md"),
        ("iteration", "1"),
        ("previous_review", "none"),
        ("repo", "sample"),
        ("branch", "muster/feat/f1"),
        ("base_branch", "main"),
        ("stage_id", "plan"),
    ] {
        v.insert(k, val.to_string());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_all_variables() {
        let vars = sample_vars();
        let tpl: String = VARIABLES
            .iter()
            .map(|v| format!("{v}={{{{{v}}}}}\n"))
            .collect();
        let out = render(&tpl, &vars).unwrap();
        assert!(out.contains("run_id=f1\n"));
        assert!(out.contains("previous_review=none\n"));
        assert!(!out.contains("{{"));
    }

    #[test]
    fn allows_inner_spaces() {
        assert_eq!(
            render("a {{ run_id }} b", &sample_vars()).unwrap(),
            "a f1 b"
        );
    }

    #[test]
    fn unknown_variable_is_error_with_line() {
        let err = render("ok\n\nhello {{nope}}", &sample_vars()).unwrap_err();
        assert_eq!(
            err,
            RenderError::UnknownVariable {
                name: "nope".into(),
                line: 3
            }
        );
    }

    #[test]
    fn leftover_braces_are_errors() {
        assert_eq!(
            render("x {{run_id", &sample_vars()).unwrap_err(),
            RenderError::Unclosed { line: 1 }
        );
        assert!(matches!(
            render("x {{ {{run_id}}", &sample_vars()).unwrap_err(),
            RenderError::Unclosed { .. }
        ));
        assert!(matches!(
            render("x {{}}", &sample_vars()).unwrap_err(),
            RenderError::UnknownVariable { .. }
        ));
    }

    #[test]
    fn missing_value_is_error() {
        let mut vars = sample_vars();
        vars.remove("repo");
        assert!(matches!(
            render("{{repo}}", &vars).unwrap_err(),
            RenderError::MissingValue { .. }
        ));
    }

    #[test]
    fn values_are_not_reexpanded() {
        let mut vars = sample_vars();
        vars.insert("repo", "{{run_id}}".into());
        assert_eq!(render("{{repo}}", &vars).unwrap(), "{{run_id}}");
    }
}
