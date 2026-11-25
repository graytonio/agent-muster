mod cli;
mod config;
mod doctor;
mod engine;
mod git;
mod herdr;
mod render;
mod state;
mod workflow;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    eprintln!("muster: {:?} is not implemented yet", cli.command);
    std::process::exit(2);
}
