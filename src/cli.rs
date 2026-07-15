use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};

use crate::core::report::CiteError;
use crate::core::{compiler, deploy, doctor, project, scaffold, uninstall, upgrade};
use colored::Colorize;

#[derive(Parser)]
#[command(
    name = "cite-cli",
    version,
    about = "Manage and enrich your podcast projects from scaffolding through deployment"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<CliCommand>,

    #[arg(global = true, short, long)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum CliCommand {
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
    project::discover_projects(&root)
}

fn load_projects(
    path: Option<String>,
    empty_msg: &str,
) -> Result<Option<Vec<project::ProjectContext>>, CiteError> {
    let roots = discover_projects(path);
    if roots.is_empty() {
        warn!("{empty_msg}");
        return Ok(None);
    }
    let mut projects = Vec::with_capacity(roots.len());
    for root in &roots {
        projects.push(load_project(root)?);
    }
    Ok(Some(projects))
}

fn print_project_header(name: &str) {
    info!("── {name} ──");
}

impl CliCommand {
    pub async fn execute(self) -> Result<(), CiteError> {
        match self {
            CliCommand::Init { name, path } => {
                let root = match path {
                    Some(p) => PathBuf::from(p).join(&name),
                    None => resolve_path(None).join(&name),
                };
                scaffold::init_project(&name, &root)?;
                println!(
                    "{}",
                    format!("Project '{name}' ready at {}", root.display()).green()
                );
                Ok(())
            }
            CliCommand::Lint { path } => {
                let Some(projects) = load_projects(path, "No projects found (no cite.toml found)")?
                else {
                    return Ok(());
                };
                let multi = projects.len() > 1;
                let mut overall_has_warnings = false;
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    }
                    let outcome = doctor::lint_all(ctx);
                    outcome.emit();
                    if outcome.has_warnings() {
                        overall_has_warnings = true;
                    }
                }
                if !overall_has_warnings {
                    println!("{}", format!("Lint complete — no issues found").green());
                }
                Ok(())
            }
            CliCommand::Build { path, force } => {
                let Some(projects) = load_projects(path, "No projects found (no cite.toml found)")?
                else {
                    return Ok(());
                };
                let multi = projects.len() > 1;
                let mut has_errors = false;
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    }
                    match compiler::compile(ctx, force).await {
                        Ok(_) => {}
                        Err(e) => {
                            error!("Build failed: {e}");
                            has_errors = true;
                        }
                    }
                }
                if has_errors {
                    Err(CiteError::Config(
                        "Build failed in one or more projects".to_string(),
                    ))
                } else {
                    println!("{}", format!("Build complete").green());
                    Ok(())
                }
            }
            CliCommand::Deploy { path, dry_run } => {
                let Some(projects) = load_projects(path, "No projects found (no cite.toml found)")?
                else {
                    return Ok(());
                };
                let multi = projects.len() > 1;
                let mut has_errors = false;
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    }
                    match deploy::deploy(ctx, dry_run).await {
                        Ok(msg) => eprintln!("{msg}"),
                        Err(e) => {
                            warn!("Deploy failed: {e}");
                            has_errors = true;
                        }
                    }
                }
                if has_errors {
                    Err(CiteError::Deploy(
                        "Deploy failed in one or more projects".to_string(),
                    ))
                } else {
                    println!("{}", format!("Deploy complete").green());
                    Ok(())
                }
            }
            CliCommand::Status { path } => {
                let Some(projects) = load_projects(path, "No projects found")? else {
                    return Ok(());
                };
                let multi = projects.len() > 1;
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    } else {
                        info!("Project Status");
                    }
                    project::print_status(ctx);
                }
                println!("{}", format!("Status complete").green());
                Ok(())
            }
            CliCommand::Doctor { path } => {
                let root = resolve_path(path.clone());
                let Some(projects) = load_projects(path, "")? else {
                    info!("Running diagnostics");
                    doctor::check_file(&root, "cite.toml", "run 'cite-cli init'");
                    doctor::check_file(&root, "metadata.yml", "");
                    return Ok(());
                };
                let multi = projects.len() > 1;
                let mut overall_has_errors = false;
                let mut overall_has_warnings = false;
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    }
                    let outcome = doctor::run(ctx)?;
                    if outcome.has_errors() {
                        overall_has_errors = true;
                    }
                    if outcome.has_warnings() {
                        overall_has_warnings = true;
                    }
                }
                if overall_has_errors {
                    return Err(CiteError::Config(
                        "Doctor found validation errors".to_string(),
                    ));
                } else if !overall_has_warnings {
                    println!(
                        "{}",
                        format!("Doctor check complete — no issues found").green()
                    );
                }
                Ok(())
            }
            CliCommand::Clean { path } => {
                let Some(projects) = load_projects(path, "No projects found")? else {
                    return Ok(());
                };
                let multi = projects.len() > 1;
                for ctx in &projects {
                    if multi {
                        print_project_header(&ctx.manifest.project.name);
                    }
                    ctx.clean()?;
                    println!("{}", format!("Cleaned build artifacts").green());
                }
                Ok(())
            }
            CliCommand::Rollback { id, path } => {
                let root = resolve_path(path);
                let ctx = load_project(&root)?;
                let msg = deploy::rollback(&ctx, &id).await?;
                info!("{msg}");
                Ok(())
            }
            CliCommand::Login {
                email,
                password,
                path,
            } => {
                let root = resolve_path(path);
                let ctx = load_project(&root)?;
                deploy::login(&ctx, email, password).await?;
                println!("{}", format!("Login complete").green());
                Ok(())
            }
            CliCommand::Upgrade => {
                let msg = upgrade::upgrade().await?;
                info!("{msg}");
                println!("{}", format!("Upgrade complete").green());
                Ok(())
            }
            CliCommand::Uninstall { force } => uninstall::uninstall(force),
        }
    }
}
