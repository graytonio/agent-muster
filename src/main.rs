use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::Parser;
use muster::{config, defaults, workflow};
use serde_json::json;

use muster::cli::{Cli, Command, GlobalArgs};
use muster::config::Config;
use muster::error::{exit, exit_code, MusterError};

fn main() {
    let cli = Cli::parse();
    let json_mode = cli.global.json;
    let code = match run(cli) {
        Ok(code) => code,
        Err(err) => {
            let code = exit_code(&err);
            if json_mode {
                println!(
                    "{}",
                    json!({"ok": false, "exit_code": code, "error": format!("{err:#}")})
                );
            } else {
                eprintln!("muster: {err:#}");
            }
            code
        }
    };
    std::process::exit(code);
}

fn run(cli: Cli) -> Result<i32> {
    let g = &cli.global;
    match cli.command {
        Command::Init { out, force } => cmd_init(g, &out, force),
        Command::ListWorkflows => cmd_list_workflows(g),
        Command::Validate { workflow } => cmd_validate(g, workflow.as_deref()),
        other => {
            bail!(MusterError::usage(format!(
                "{other:?} is not implemented yet"
            )))
        }
    }
}

pub fn config_dir(g: &GlobalArgs) -> PathBuf {
    config::resolve_config_dir(
        g.config_dir.as_deref(),
        std::env::var_os("MUSTER_CONFIG_DIR"),
    )
}

/// Print `value` as JSON, or run `human` to print text.
pub fn emit(g: &GlobalArgs, value: serde_json::Value, human: impl FnOnce()) {
    if g.json {
        println!("{value}");
    } else {
        human();
    }
}

fn cmd_init(g: &GlobalArgs, out: &Path, force: bool) -> Result<i32> {
    let live = config_dir(g);
    if let (Ok(a), Ok(b)) = (out.canonicalize(), live.canonicalize()) {
        if a == b {
            bail!(MusterError::usage(format!(
                "refusing to write into the live config dir {}",
                live.display()
            )));
        }
    }
    if out.exists() {
        let non_empty = std::fs::read_dir(out)?.next().is_some();
        if non_empty && !force {
            bail!(MusterError::usage(format!(
                "{} is not empty (use --force to overwrite)",
                out.display()
            )));
        }
    }
    let mut written = Vec::new();
    for (rel, contents) in defaults::FILES {
        let path = out.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, contents)?;
        written.push(path.display().to_string());
    }
    emit(g, json!({"ok": true, "out": out, "files": written}), || {
        println!("wrote {} files to {}", written.len(), out.display());
    });
    Ok(exit::OK)
}

fn cmd_list_workflows(g: &GlobalArgs) -> Result<i32> {
    let dir = config_dir(g);
    let names = workflow::list(&dir)?;
    let mut rows = Vec::new();
    for name in names {
        match workflow::load(&dir, &name) {
            Ok(l) => rows.push(json!({
                "name": name,
                "description": l.workflow.workflow.description,
                "stages": l.workflow.stages.iter().map(|s| &s.id).collect::<Vec<_>>(),
                "valid": true,
            })),
            Err(e) => rows.push(json!({"name": name, "valid": false, "error": format!("{e:#}")})),
        }
    }
    emit(g, json!({"ok": true, "workflows": rows}), || {
        for r in &rows {
            let name = r["name"].as_str().unwrap_or("");
            if r["valid"].as_bool() == Some(true) {
                let stages: Vec<&str> = r["stages"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|s| s.as_str()).collect())
                    .unwrap_or_default();
                println!(
                    "{name:<14} {}  [{}]",
                    r["description"].as_str().unwrap_or(""),
                    stages.join(" → ")
                );
            } else {
                println!("{name:<14} INVALID (run `muster validate {name}`)");
            }
        }
    });
    Ok(exit::OK)
}

fn cmd_validate(g: &GlobalArgs, only: Option<&str>) -> Result<i32> {
    let dir = config_dir(g);
    let mut problems = Vec::new();
    let mut checked = Vec::new();
    if let Err(e) = Config::load(&dir) {
        problems.push(format!("{e:#}"));
    }
    let names = match only {
        Some(n) => vec![n.to_string()],
        None => match workflow::list(&dir) {
            Ok(n) => n,
            Err(e) => {
                problems.push(format!("{e:#}"));
                Vec::new()
            }
        },
    };
    if only.is_none() && names.is_empty() && problems.is_empty() {
        problems.push(format!(
            "no workflows in {}",
            dir.join(config::WORKFLOWS_DIR).display()
        ));
    }
    for name in &names {
        match workflow::load(&dir, name) {
            Ok(_) => checked.push(name.clone()),
            Err(e) => problems.push(format!("{e:#}")),
        }
    }
    let ok = problems.is_empty();
    emit(
        g,
        json!({"ok": ok, "config_dir": dir, "workflows": checked, "errors": problems}),
        || {
            for p in &problems {
                eprintln!("error: {p}");
            }
            if ok {
                println!(
                    "ok: {} and {} workflow(s) valid ({})",
                    config::CONFIG_FILE,
                    checked.len(),
                    checked.join(", ")
                );
            }
        },
    );
    Ok(if ok { exit::OK } else { exit::USAGE })
}
