use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::{Path, PathBuf};
use tracing::debug;

use crate::error::CiteError;
use crate::{compiler, deploy, project, scaffold, uninstall, upgrade, validation};

#[derive(Parser)]
#[command(
    name = "cite-cli",
    version,
    about = "Manage and enrich your podcast projects from scaffolding through deployment"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[arg(global = true, short, long)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a new project with recommended structure and starter files
    Init {
        name: String,
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Run full validation (structure, files, metadata, cross-references)
    Validate {
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Run linting rules (naming, style, word counts)
    Lint {
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Execute the compiler protocol and produce a build artifact
    Build {
        #[arg(short, long)]
        path: Option<String>,
        #[arg(long)]
        force: bool,
    },
    /// Deploy the built project to Supabase staging
    Deploy {
        #[arg(short, long)]
        path: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Show project health, validation summary, and sync state
    Status {
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Diagnose common project issues and configuration problems
    Doctor {
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Remove build artifacts, temporary files, and build cache
    Clean {
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Rollback a deployment by its unique ID
    Rollback {
        id: String,
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Self-update to the latest GitHub release
    Upgrade,
    /// Remove cite-cli binary and clean up shell configuration
    Uninstall {
        #[arg(short, long)]
        force: bool,
    },
}

fn resolve_path(path: Option<String>) -> PathBuf {
    match path {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    }
}

fn load_project(root: &Path) -> Result<project::ProjectContext, CiteError> {
    debug!(path = %root.display(), "Loading project context");
    project::ProjectContext::load(root)
}

fn discover_projects(path: Option<String>) -> Vec<PathBuf> {
    let root = resolve_path(path);
    let mut projects = Vec::new();

    if root.join("cite.toml").exists() {
        projects.push(root.clone());
    }

    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() && p != root && p.join("cite.toml").exists() {
                projects.push(p);
            }
        }
    }

    projects.sort();
    projects
}

fn load_projects(
    path: Option<String>,
    empty_msg: &str,
) -> Result<Option<(Vec<project::ProjectContext>, bool)>, CiteError> {
    let roots = discover_projects(path);
    if roots.is_empty() {
        eprintln!("{empty_msg}");
        return Ok(None);
    }
    let multi = roots.len() > 1;
    let mut projects = Vec::with_capacity(roots.len());
    for root in &roots {
        projects.push(load_project(root)?);
    }
    Ok(Some((projects, multi)))
}

enum Style {
    Success,
    Error,
    Warning,
    Header,
}

fn styled(msg: impl AsRef<str>, style: Style) -> String {
    let s = msg.as_ref();
    match style {
        Style::Success => format!("  {}", s.green().bold()),
        Style::Error => format!("  {}", s.red().bold()),
        Style::Warning => format!("  {}", s.yellow().bold()),
        Style::Header => s.bold().underline().to_string(),
    }
}

fn print_project_header(name: &str) {
    eprintln!("── {} ──", name);
}

fn check_file(root: &Path, filename: &str, hint: &str) {
    let path = root.join(filename);
    if path.exists() {
        eprintln!("{}", styled(format!("{filename} found"), Style::Success));
    } else if hint.is_empty() {
        eprintln!("{}", styled(format!("{filename} not found"), Style::Error));
    } else {
        eprintln!(
            "{}",
            styled(format!("{filename} not found - {hint}"), Style::Error)
        );
    }
}

impl Command {
    pub async fn execute(self) -> Result<(), CiteError> {
        match self {
            Command::Init { name, path } => {
                let root = match path {
                    Some(p) => PathBuf::from(p),
                    None => std::env::current_dir()
                        .unwrap_or_else(|_| PathBuf::from("."))
                        .join(&name),
                };
                let report = scaffold::init_project(&name, &root)?;

                for d in &report.directories_created {
                    eprintln!("  {} (created)", d);
                }
                for f in &report.files_created {
                    eprintln!("  {} (created)", f);
                }
                for f in &report.files_skipped {
                    eprintln!("  {} (skipped)", f);
                }
                eprintln!(
                    "{}",
                    styled(
                        format!("Project '{name}' ready at {}", root.display()),
                        Style::Success
                    )
                );
                Ok(())
            }
            Command::Validate { path } => {
                let Some((projects, multi)) =
                    load_projects(path, "No projects found (no cite.toml found)")?
                else {
                    return Ok(());
                };
                let mut has_errors = false;
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    }
                    let report = validation::validate_all(ctx);
                    report.print();
                    if report.has_errors() {
                        has_errors = true;
                    }
                }
                if has_errors {
                    Err(CiteError::Validation("Validation failed".to_string()))
                } else {
                    Ok(())
                }
            }
            Command::Lint { path } => {
                let Some((projects, multi)) =
                    load_projects(path, "No projects found (no cite.toml found)")?
                else {
                    return Ok(());
                };
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    }
                    let report = validation::lint_all(ctx);
                    report.print();
                }
                Ok(())
            }
            Command::Build { path, force } => {
                let Some((projects, multi)) =
                    load_projects(path, "No projects found (no cite.toml found)")?
                else {
                    return Ok(());
                };
                let mut has_errors = false;
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    }
                    match compiler::compile(ctx, force).await {
                        Ok(report) => report.print(),
                        Err(e) => {
                            eprintln!("{}", styled(format!("Build failed: {e}"), Style::Error));
                            has_errors = true;
                        }
                    }
                }
                if has_errors {
                    Err(CiteError::Deploy(
                        "Build failed in one or more projects".to_string(),
                    ))
                } else {
                    Ok(())
                }
            }
            Command::Deploy { path, dry_run } => {
                let Some((projects, multi)) =
                    load_projects(path, "No projects found (no cite.toml found)")?
                else {
                    return Ok(());
                };
                let mut has_errors = false;
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    }
                    if let Err(e) = deploy::deploy(ctx, dry_run).await {
                        eprintln!("{}", styled(format!("Deploy failed: {e}"), Style::Error));
                        has_errors = true;
                    }
                }
                if has_errors {
                    Err(CiteError::Deploy(
                        "Deploy failed in one or more projects".to_string(),
                    ))
                } else {
                    Ok(())
                }
            }
            Command::Status { path } => {
                let Some((projects, multi)) = load_projects(path, "No projects found")? else {
                    return Ok(());
                };
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    } else {
                        eprintln!("{}", styled("Project Status", Style::Header));
                    }
                    eprintln!("  Name: {}", ctx.manifest.project.name);
                    eprintln!("  Project Root: {}", ctx.root.display());
                    eprintln!("  Artist ID: {}", ctx.manifest.project.artist_id);
                    if let Some(backend) = &ctx.manifest.backend {
                        eprintln!("  Active subscription: {}", backend.subscription_plan);
                        eprintln!("  Publishing to: {}", backend.staging_url);
                    }
                    eprintln!("  Podcasts: {}", ctx.metadata.podcasts.len());
                    let build_path = ctx.build_dir().join("content.json");
                    if build_path.exists() {
                        eprintln!("  Build: {}", styled("exists", Style::Success));
                        if let Ok(meta) = std::fs::metadata(&build_path)
                            && let Ok(modified) = meta.modified()
                            && let Ok(elapsed) = modified.elapsed()
                        {
                            let secs = elapsed.as_secs();
                            let since = if secs < 60 {
                                "just now".to_string()
                            } else if secs < 3600 {
                                format!("{}m ago", secs / 60)
                            } else {
                                format!("{}h ago", secs / 3600)
                            };
                            eprintln!("  Built: {since}");
                        }
                    } else {
                        eprintln!("  Build: not built");
                    }
                }
                Ok(())
            }
            Command::Doctor { path } => {
                let root = resolve_path(path.clone());
                let Some((projects, multi)) = load_projects(path, "")? else {
                    eprintln!("{}", styled("Running diagnostics", Style::Header));
                    check_file(&root, "cite.toml", "run 'cite-cli init'");
                    check_file(&root, "metadata.yml", "");
                    return Ok(());
                };
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    } else {
                        eprintln!("{}", styled("Running diagnostics", Style::Header));
                    }
                    check_file(&ctx.root, "cite.toml", "run 'cite-cli init'");
                    check_file(&ctx.root, "metadata.yml", "");
                    for dir in &["content", "assets/audio", "assets/images", "build"] {
                        let d = ctx.root.join(dir);
                        if d.is_dir() {
                            eprintln!("{}", styled(format!("{dir}/ exists"), Style::Success));
                        } else if *dir == "build" {
                            eprintln!("  {dir}/ missing (created by build)");
                        } else {
                            eprintln!("  {dir}/ missing (will be created on init)");
                        }
                    }
                    if ctx.manifest.backend.is_some() {
                        eprintln!(
                            "{}",
                            styled("Backend configured for staging", Style::Success)
                        );
                    } else {
                        eprintln!("  No backend configured (deploy will fail)");
                    }
                    if ctx.manifest.build.incremental {
                        eprintln!("{}", styled("Incremental builds enabled", Style::Success));
                    }
                    if ctx.manifest.project.artist_id.is_empty() {
                        eprintln!(
                            "{}",
                            styled(
                                "Artist ID is empty - set it in [project] in cite.toml",
                                Style::Warning
                            )
                        );
                    } else {
                        eprintln!("  Artist ID: {}", ctx.manifest.project.artist_id);
                    }
                    if std::env::var("CITE_STAGING_SERVICE_KEY").is_ok() {
                        eprintln!(
                            "{}",
                            styled("CITE_STAGING_SERVICE_KEY env var set", Style::Success)
                        );
                    } else if ctx
                        .manifest
                        .backend
                        .as_ref()
                        .map(|b| !b.staging_service_key.is_empty())
                        .unwrap_or(false)
                    {
                        eprintln!("  Using inline staging_service_key from cite.toml");
                    } else {
                        eprintln!(
                            "{}",
                            styled(
                                "No staging service key found - deploy will fail",
                                Style::Warning
                            )
                        );
                    }
                }
                Ok(())
            }
            Command::Clean { path } => {
                let Some((projects, multi)) = load_projects(path, "No projects found")? else {
                    return Ok(());
                };
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    }
                    ctx.clean()?;
                    eprintln!("{}", styled("Cleaned build artifacts", Style::Success));
                }
                Ok(())
            }
            Command::Rollback { id, path } => {
                let root = resolve_path(path);
                let ctx = load_project(&root)?;
                deploy::rollback(&ctx, &id).await
            }
            Command::Upgrade => upgrade::upgrade().await,
            Command::Uninstall { force } => uninstall::uninstall(force),
        }
    }
}
