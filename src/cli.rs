use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use tracing::debug;

use crate::error::CiteError;
use crate::{compiler, deploy, doctor, project, scaffold, uninstall, upgrade};

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
    /// Authenticate with Supabase and store a session for user-scoped deploys
    Login {
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        password: Option<String>,
        #[arg(short, long)]
        path: Option<String>,
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

fn print_project_header(name: &str) {
    eprintln!("── {} ──", name);
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
                    doctor::styled(
                        format!("Project '{name}' ready at {}", root.display()),
                        doctor::Style::Success
                    )
                );
                Ok(())
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
                    let report = doctor::lint_all(ctx);
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
                            eprintln!(
                                "{}",
                                doctor::styled(format!("Build failed: {e}"), doctor::Style::Error)
                            );
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
                        eprintln!(
                            "{}",
                            doctor::styled(format!("Deploy failed: {e}"), doctor::Style::Error)
                        );
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
                        eprintln!(
                            "{}",
                            doctor::styled("Project Status", doctor::Style::Header)
                        );
                    }
                    eprintln!("  Name: {}", ctx.manifest.project.name);
                    eprintln!("  Project Root: {}", ctx.root.display());
                    eprintln!("  Artist ID: {}", ctx.manifest.project.artist_id);
                    if let Some(backend) = &ctx.manifest.backend {
                        eprintln!("  Publishing to: {}", backend.staging_url);
                    }
                    eprintln!("  Podcasts: {}", ctx.metadata.podcasts.len());
                    let build_path = ctx.build_dir().join("content.json");
                    if build_path.exists() {
                        eprintln!(
                            "  Build: {}",
                            doctor::styled("exists", doctor::Style::Success)
                        );
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
                    eprintln!(
                        "{}",
                        doctor::styled("Running diagnostics", doctor::Style::Header)
                    );
                    doctor::check_file(&root, "cite.toml", "run 'cite-cli init'");
                    doctor::check_file(&root, "metadata.yml", "");
                    return Ok(());
                };
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    }
                    if doctor::run(ctx) {
                        return Err(CiteError::Validation(
                            "Doctor found validation errors".to_string(),
                        ));
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
                    eprintln!(
                        "{}",
                        doctor::styled("Cleaned build artifacts", doctor::Style::Success)
                    );
                }
                Ok(())
            }
            Command::Rollback { id, path } => {
                let root = resolve_path(path);
                let ctx = load_project(&root)?;
                deploy::rollback(&ctx, &id).await
            }
            Command::Login {
                email,
                password,
                path,
            } => {
                let root = resolve_path(path);
                let ctx = load_project(&root)?;
                deploy::login(&ctx, email, password).await
            }
            Command::Upgrade => upgrade::upgrade().await,
            Command::Uninstall { force } => uninstall::uninstall(force),
        }
    }
}
