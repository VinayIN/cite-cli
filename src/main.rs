mod cli;
mod core;
mod tui;

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
        .with_writer(std::io::stderr)
        .init();

    info!("cite-cli v{}", env!("CARGO_PKG_VERSION"));

    if let Some(cmd) = cli.command {
        if let Err(e) = cmd.execute().await {
            eprintln!("{} {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    } else {
        if let Err(e) = tui::run_tui().await {
            eprintln!("{} {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}
