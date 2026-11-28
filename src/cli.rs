//! Command-line surface. Parsing only; behavior lives in the other modules.

use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "muster",
    version,
    about = "Orchestrate multi-agent workflows on a herdr server",
    long_about = None,
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Args, Clone)]
pub struct GlobalArgs {
    /// Read-only config directory (default: $MUSTER_CONFIG_DIR, then /etc/muster)
    #[arg(long, global = true, value_name = "PATH")]
    pub config_dir: Option<PathBuf>,

    /// Print machine-readable JSON instead of human output
    #[arg(long, global = true)]
    pub json: bool,

    /// Verbose output (log every external command)
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Write the default config tree (muster.toml, workflows/, prompts/) to a directory
    Init {
        /// Directory to write into
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// Write into a non-empty directory, overwriting files with the same names
        #[arg(long)]
        force: bool,
    },
    /// Check that the runtime contract is satisfied
    Doctor,
    /// List the workflows in the config directory
    ListWorkflows,
    /// Load and validate config, workflows and prompts (no herdr, git or network)
    Validate {
        /// Validate only this workflow (default: all)
        workflow: Option<String>,
    },
    /// Start a workflow run
    Start(StartArgs),
    /// Continue a run that needs a human, from its recorded stage
    Resume { run_id: String },
    /// Show all runs, or the detail and event log for one run
    Status { run_id: Option<String> },
    /// Close a run's panes and remove its worktree
    Clean {
        run_id: String,
        /// Also delete the run's local branch
        #[arg(long)]
        delete_branch: bool,
        /// Clean even if the run is still running
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Args, Clone)]
#[command(group(
    ArgGroup::new("brief_source")
        .required(true)
        .multiple(false)
        .args(["brief", "brief_file", "brief_stdin"])
))]
pub struct StartArgs {
    /// Workflow name (workflows/<name>.toml)
    pub workflow: String,

    /// Repo name, a directory under repos_dir
    #[arg(long)]
    pub repo: String,

    /// The brief text itself (its first line becomes the PR title)
    #[arg(long, value_name = "TEXT")]
    pub brief: Option<String>,

    /// Read the brief from this file
    #[arg(long, value_name = "PATH")]
    pub brief_file: Option<PathBuf>,

    /// Read the brief from standard input
    #[arg(long)]
    pub brief_stdin: bool,

    /// Use this run id instead of generating one
    #[arg(long)]
    pub run_id: Option<String>,

    /// Skip an optional stage (repeatable)
    #[arg(long, value_name = "STAGE")]
    pub skip: Vec<String>,

    /// Run in the background, logging to <log_dir>/<run>.log, and print the run id
    #[arg(long)]
    pub detach: bool,
}
