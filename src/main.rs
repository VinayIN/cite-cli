mod cache;
mod cli;
mod compiler;
mod deploy;
mod error;
mod manifest;
mod metadata;
mod project;
mod scaffold;
mod slug;
mod uninstall;
mod upgrade;
mod validation;

use clap::Parser;
use cli::{Cli, Command};
use colored::Colorize;
use std::path::{Path, PathBuf};
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

fn resolve_path(path: Option<String>) -> PathBuf {
    match path {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    }
}

fn load_project(root: &Path) -> Result<project::ProjectContext, error::CiteError> {
    debug!(path = %root.display(), "Loading project context");
    project::ProjectContext::load(root)
}

async fn run(cli: &Cli) -> Result<(), error::CiteError> {
    match &cli.command {
        Command::Init { name, path } => {
            let root = match path {
                Some(p) => PathBuf::from(p),
                None => std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(name),
            };
            let name = name.clone();
            scaffold::init_project(&name, &root)?;
            eprintln!(
                "{}",
                format!("✔ Initialized project '{}' at {}", name, root.display())
                    .green()
                    .bold()
            );
            Ok(())
        }
        Command::Validate { path } => {
            let root = resolve_path(path.clone());
            let ctx = load_project(&root)?;
            let report = validation::validate_all(&ctx);
            report.print();
            if report.has_errors() {
                Err(error::CiteError::Validation(
                    "Validation failed".to_string(),
                ))
            } else {
                Ok(())
            }
        }
        Command::Lint { path } => {
            let root = resolve_path(path.clone());
            let ctx = load_project(&root)?;
            let report = validation::lint_all(&ctx);
            report.print();
            Ok(())
        }
        Command::Build { path, force } => {
            let root = resolve_path(path.clone());
            let ctx = load_project(&root)?;
            let report = compiler::build(&ctx, *force).await?;
            report.print();
            Ok(())
        }
        Command::Deploy { path, dry_run } => {
            let root = resolve_path(path.clone());
            let ctx = load_project(&root)?;
            deploy::deploy(&ctx, *dry_run).await
        }
        Command::Status { path } => {
            let root = resolve_path(path.clone());
            let ctx = load_project(&root)?;
            eprintln!("{}", "Project Status".bold().underline());
            eprintln!("  Name:     {}", ctx.manifest.project.name);
            eprintln!("  Version:  {}", ctx.manifest.project.version);
            eprintln!("  Root:     {}", root.display());
            eprintln!("  Artists:  {}", ctx.metadata.artists.len());
            eprintln!("  News:     {}", ctx.metadata.news.len());
            eprintln!("  Podcasts: {}", ctx.metadata.podcasts.len());
            eprintln!("  Newsletters: {}", ctx.metadata.newsletters.len());
            let build_dir = ctx.build_dir();
            if build_dir.join("content.json").exists() {
                eprintln!("  Build:    {} (exists)", "✔".green());
            } else {
                eprintln!("  Build:    {} (not built)", "✖".red());
            }
            Ok(())
        }
        Command::Doctor { path } => {
            let root = resolve_path(path.clone());
            eprintln!("{}", "Running diagnostics...".bold());

            let manifest_path = root.join("cite.toml");
            if manifest_path.exists() {
                eprintln!("  {} cite.toml found", "✔".green());
            } else {
                eprintln!("  {} cite.toml not found - run 'cite-cli init'", "✖".red());
            }

            let meta_path = root.join("metadata.yml");
            if meta_path.exists() {
                eprintln!("  {} metadata.yml found", "✔".green());
            } else {
                eprintln!("  {} metadata.yml not found", "✖".red());
            }

            for dir in &["content", "assets/audio", "assets/images", "build"] {
                let d = root.join(dir);
                if d.is_dir() {
                    eprintln!("  {} {dir}/ exists", "✔".green());
                } else {
                    eprintln!("  {} {dir}/ missing (will be created on init)", "ℹ".cyan());
                }
            }

            if let Ok(ctx) = load_project(&root) {
                if ctx.manifest.backend.is_some() {
                    eprintln!("  {} Backend configured for staging", "✔".green());
                } else {
                    eprintln!("  {} No backend configured (deploy will fail)", "ℹ".cyan());
                }
                if ctx.manifest.build.incremental {
                    eprintln!("  {} Incremental builds enabled", "✔".green());
                }
            }

            Ok(())
        }
        Command::Clean { path } => {
            let root = resolve_path(path.clone());
            scaffold::clean_project(&root)?;
            eprintln!("{}", "✔ Cleaned build artifacts".green().bold());
            Ok(())
        }
        Command::Rollback { id, path } => {
            let root = resolve_path(path.clone());
            let ctx = load_project(&root)?;
            deploy::rollback(&ctx, id).await
        }
        Command::Upgrade => upgrade::upgrade().await,
        Command::Uninstall { force } => uninstall::uninstall(*force),
    }
}

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

    if let Err(e) = run(&cli).await {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}
