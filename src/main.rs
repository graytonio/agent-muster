use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::Parser;
use muster::{config, defaults, workflow};
use serde_json::json;

use muster::cli::{Cli, Command, GlobalArgs, StartArgs};
use muster::config::Config;
use muster::doctor;
use muster::engine::{self, Brief, Engine, Outcome, StartRequest};
use muster::error::{exit, exit_code, MusterError};
use muster::herdr::CliHerdr;
use muster::state::{RunStatus, StateStore};

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
        Command::Doctor => cmd_doctor(g),
        Command::Start(args) => cmd_start(g, args),
        Command::Resume { run_id } => cmd_resume(g, &run_id),
        Command::Status { run_id } => cmd_status(g, run_id.as_deref()),
        Command::Clean {
            run_id,
            delete_branch,
            force,
        } => cmd_clean(g, &run_id, delete_branch, force),
    }
}

fn load_config(g: &GlobalArgs) -> Result<Config> {
    Config::load(&config_dir(g))
}

fn real_herdr(cfg: &Config, g: &GlobalArgs) -> CliHerdr {
    CliHerdr::new(cfg.herdr_bin.clone(), cfg.herdr_session.clone(), g.verbose)
}

fn print_outcome(g: &GlobalArgs, o: &Outcome) -> i32 {
    let code = o.exit_code();
    let mut v = serde_json::to_value(o).unwrap_or_default();
    v["ok"] = json!(code == exit::OK);
    v["exit_code"] = json!(code);
    emit(g, v, || match o.status {
        RunStatus::Done => match &o.pr_url {
            Some(url) => println!("{}: done, PR {url}", o.run_id),
            None => println!(
                "{}: done, artifacts in {}",
                o.run_id,
                o.run_dir
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ),
        },
        RunStatus::NeedsHuman => println!(
            "{}: needs a human: {}\nresume with: muster resume {}",
            o.run_id,
            o.reason.as_deref().unwrap_or(""),
            o.run_id
        ),
        RunStatus::Failed => println!(
            "{}: failed: {}",
            o.run_id,
            o.reason.as_deref().unwrap_or("")
        ),
        RunStatus::Running => println!("{}: running", o.run_id),
    });
    code
}

/// Map the (mutually exclusive) brief flags to a `Brief`.
fn brief_source(args: &StartArgs) -> Brief {
    match (&args.brief, &args.brief_file) {
        (Some(t), _) => Brief::Text(t.clone()),
        (None, Some(p)) => Brief::File(p.clone()),
        (None, None) => Brief::Stdin,
    }
}

fn cmd_start(g: &GlobalArgs, args: StartArgs) -> Result<i32> {
    let cfg = load_config(g)?;
    if args.detach {
        return detach(g, &cfg, args);
    }
    let brief = brief_source(&args).read()?;
    let herdr = real_herdr(&cfg, g);
    let engine = Engine::new(&cfg, &herdr, g.verbose, !g.json);
    let outcome = engine.start(StartRequest {
        workflow: args.workflow,
        repo: args.repo,
        brief,
        run_id: args.run_id,
        skip: args.skip,
    })?;
    Ok(print_outcome(g, &outcome))
}

/// Re-exec `muster start` in the background with a fixed run id, logging to
/// `<log_dir>/<run>.log`.
fn detach(g: &GlobalArgs, cfg: &Config, args: StartArgs) -> Result<i32> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    let store = StateStore::new(cfg.state_dir.clone());
    store.ensure()?;
    let id = match &args.run_id {
        Some(id) => id.clone(),
        None => store.next_run_id(&args.workflow)?,
    };
    muster::state::check_run_id(&id)?;
    if store.exists(&id) {
        bail!(MusterError::usage(format!("run {id} already exists")));
    }
    std::fs::create_dir_all(&cfg.log_dir)?;
    let brief_path = match &args.brief_file {
        Some(p) => std::fs::canonicalize(p)
            .map_err(|e| MusterError::usage(format!("cannot read brief {}: {e}", p.display())))?,
        None => {
            let text = brief_source(&args).read()?;
            let p = cfg.log_dir.join(format!("{id}.brief.md"));
            std::fs::write(&p, text)?;
            p
        }
    };
    let log_path = cfg.log_dir.join(format!("{id}.log"));
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.arg("--config-dir").arg(&cfg.config_dir);
    if g.verbose {
        cmd.arg("-v");
    }
    cmd.args([
        "start",
        &args.workflow,
        "--repo",
        &args.repo,
        "--run-id",
        &id,
    ])
    .arg("--brief-file")
    .arg(&brief_path);
    for s in &args.skip {
        cmd.args(["--skip", s]);
    }
    cmd.stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .process_group(0);
    let child = cmd.spawn()?;
    emit(
        g,
        json!({"ok": true, "run_id": id, "pid": child.id(), "log": log_path}),
        || println!("{id}"),
    );
    Ok(exit::OK)
}

fn cmd_resume(g: &GlobalArgs, id: &str) -> Result<i32> {
    let cfg = load_config(g)?;
    let herdr = real_herdr(&cfg, g);
    let engine = Engine::new(&cfg, &herdr, g.verbose, !g.json);
    let outcome = engine.resume(id)?;
    Ok(print_outcome(g, &outcome))
}

fn cmd_status(g: &GlobalArgs, id: Option<&str>) -> Result<i32> {
    let cfg = load_config(g)?;
    let store = StateStore::new(cfg.state_dir.clone());
    match id {
        None => {
            let rows = engine::status_rows(&store)?;
            emit(g, json!({"ok": true, "runs": rows}), || {
                if rows.is_empty() {
                    println!("no runs");
                    return;
                }
                println!(
                    "{:<8} {:<12} {:<20} {:<12} {:>4}  {:<12} {:>5}",
                    "ID", "WORKFLOW", "REPO", "STAGE", "ITER", "STATUS", "AGE"
                );
                for r in &rows {
                    let status = if r.stale {
                        format!("{} (stale)", r.status)
                    } else {
                        r.status.clone()
                    };
                    println!(
                        "{:<8} {:<12} {:<20} {:<12} {:>4}  {:<12} {:>5}",
                        r.id, r.workflow, r.repo, r.stage, r.iteration, status, r.age
                    );
                }
            });
        }
        Some(id) => {
            let mut v = engine::status_detail(&store, &cfg.config_dir, id)?;
            v["ok"] = json!(true);
            let human = v.clone();
            emit(g, v, || print_detail(&human));
        }
    }
    Ok(exit::OK)
}

fn print_detail(v: &serde_json::Value) {
    let s = |k: &str| v[k].as_str().unwrap_or("-").to_string();
    println!("run       {}", s("id"));
    println!("workflow  {}", s("workflow"));
    println!("repo      {}", s("repo"));
    println!("branch    {}", s("branch"));
    let status = if v["stale"].as_bool() == Some(true) {
        format!(
            "{} (stale: no process is driving it; resume it)",
            s("status")
        )
    } else {
        s("status")
    };
    println!("status    {status}");
    if let Some(r) = v["reason"].as_str() {
        println!("reason    {r}");
    }
    println!(
        "stage     {} (iteration {})",
        v["cursor"]["stage"].as_str().unwrap_or("finish"),
        v["cursor"]["iteration"]
    );
    println!("worktree  {}", s("worktree_path"));
    if let Some(u) = v["pr_url"].as_str() {
        println!("pr        {u}");
    }
    println!("age       {}", s("age"));
    match v["live_config_changed"].as_bool() {
        Some(true) => println!(
            "config    live config has changed since this run started (the run keeps its snapshot)"
        ),
        Some(false) => println!("config    unchanged since start"),
        None => println!("config    live config can no longer be read for this workflow"),
    }
    println!("\nevents:");
    for e in v["events"].as_array().into_iter().flatten() {
        let stage = match (e["stage"].as_str(), e["iteration"].as_u64()) {
            (Some(st), Some(i)) => format!("{st}#{i}"),
            (Some(st), None) => st.to_string(),
            _ => String::new(),
        };
        println!(
            "  {}  {:<12} {:<10} {}",
            e["at"].as_str().unwrap_or(""),
            e["kind"].as_str().unwrap_or(""),
            stage,
            e["message"].as_str().unwrap_or("")
        );
    }
}

fn cmd_clean(g: &GlobalArgs, id: &str, delete_branch: bool, force: bool) -> Result<i32> {
    let cfg = load_config(g)?;
    let herdr = real_herdr(&cfg, g);
    let engine = Engine::new(&cfg, &herdr, g.verbose, !g.json);
    let v = engine.clean(id, delete_branch, force)?;
    let human = v.clone();
    emit(g, v, || {
        for a in human["actions"].as_array().into_iter().flatten() {
            println!("{}", a.as_str().unwrap_or(""));
        }
        println!("cleaned {id}");
    });
    Ok(exit::OK)
}

fn cmd_doctor(g: &GlobalArgs) -> Result<i32> {
    let report = doctor::run(&config_dir(g), g.verbose);
    let ok = report.ok();
    let human = report.clone();
    emit(g, serde_json::to_value(&report)?, || human.print());
    Ok(if ok { exit::OK } else { exit::FAILED })
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
