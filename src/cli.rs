use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{error, info, instrument, warn};

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

#[instrument]
fn load_projects(
    path: Option<String>,
    empty_msg: &str,
) -> Result<Option<Vec<project::ProjectContext>>, CiteError> {
    let root = match path {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    let mut roots = project::discover_projects(&root);
    roots.sort();
    if roots.is_empty() {
        warn!("{empty_msg}");
        return Ok(None);
    }
    let mut projects = Vec::with_capacity(roots.len());
    for root in &roots {
        projects.push(project::ProjectContext::load(root)?);
    }
    Ok(Some(projects))
}

impl CliCommand {
    pub async fn execute(self) -> Result<(), CiteError> {
        match self {
            CliCommand::Init { name, path } => {
                let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                let root = match path {
                    Some(p) => PathBuf::from(p).join(&name),
                    None => cwd.join(&name),
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
                        println!("{}", format!("── {} ──", ctx.manifest.project.name).green());
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
                        println!("{}", format!("── {} ──", ctx.manifest.project.name).green());
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
                        println!("{}", format!("── {} ──", ctx.manifest.project.name).green());
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
                        println!("{}", format!("── {} ──", ctx.manifest.project.name).green());
                    } else {
                        info!("Project Status");
                    }
                    project::print_status(ctx);
                }
                println!("{}", format!("Status complete").green());
                Ok(())
            }
            CliCommand::Doctor { path } => {
                let root = match path.clone() {
                    Some(p) => PathBuf::from(p),
                    None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                };
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
                        println!("{}", format!("── {} ──", ctx.manifest.project.name).green());
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
                        println!("{}", format!("── {} ──", ctx.manifest.project.name).green());
                    }
                    ctx.clean()?;
                    println!("{}", format!("Cleaned build artifacts").green());
                }
                Ok(())
            }
            CliCommand::Rollback { id, path } => {
                let root = match path {
                    Some(p) => PathBuf::from(p),
                    None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                };
                let ctx = project::ProjectContext::load(&root)?;
                let msg = deploy::rollback(&ctx, &id).await?;
                info!("{msg}");
                Ok(())
            }
            CliCommand::Login {
                email,
                password,
                path,
            } => {
                let root = match path {
                    Some(p) => PathBuf::from(p),
                    None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                };
                let ctx = project::ProjectContext::load(&root)?;
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
