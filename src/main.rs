mod cache;
mod cli;
mod compiler;
mod deploy;
mod error;
mod manifest;
mod metadata;
mod project;
mod scaffold;
mod uninstall;
mod upgrade;
mod validation;

use clap::Parser;
use cli::Cli;
use colored::Colorize;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        EnvFilter::new("cite_cli=trace")
    } else {
        EnvFilter::new("cite_cli=info")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_level(true)
        .without_time()
        .init();

    info!("cite-cli v{}", env!("CARGO_PKG_VERSION"));

    if let Err(e) = cli.command.execute().await {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}
