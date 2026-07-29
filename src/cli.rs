use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{error, info, instrument, warn};

use crate::core::report::CiteError;
use crate::core::{compiler, deploy, doctor, project, scaffold, uninstall, upgrade};
use colored::Colorize;

fn print_json<T: serde::Serialize>(value: &T) {
    if let Ok(json) = serde_json::to_string_pretty(value) {
        println!("{json}");
    }
}

#[derive(Clone, Parser)]
#[command(
    name = "cite-cli",
    version,
    about = "Create, validate, build, and deploy podcast content to Supabase"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<CliCommand>,

    #[arg(global = true, long, default_value = ".")]
    pub path: String,

    #[arg(global = true, short, long)]
    pub verbose: bool,

    #[arg(global = true, short, long)]
    pub quiet: bool,

    #[arg(global = true, long)]
    pub json: bool,

    #[arg(global = true, long)]
    pub dry_run: bool,

    #[arg(global = true, long)]
    pub tui: bool,
}

#[derive(Clone, Subcommand)]
pub enum CliCommand {
    Init {
        name: String,
    },
    Lint,
    Build {
        #[arg(long)]
        force: bool,
    },
    Deploy,
    Login {
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        password: Option<String>,
    },
    Status,
    Doctor,
    Clean,
    Rollback {
        id: String,
    },
    Upgrade,
    Uninstall {
        #[arg(short, long)]
        force: bool,
    },
}

#[instrument]
fn load_projects(
    path: &str,
    empty_msg: &str,
) -> Result<Option<Vec<project::ProjectContext>>, CiteError> {
    let root = PathBuf::from(path);
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
    pub async fn execute(self, cli: &Cli) -> Result<(), CiteError> {
        let path = &cli.path;
        match self {
            CliCommand::Init { name } => {
                let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                let root = PathBuf::from(path).join(&name);
                let root = if root.is_absolute() {
                    root
                } else {
                    cwd.join(&root)
                };
                scaffold::init_project(&name, &root)?;
                if cli.json {
                    print_json(&serde_json::json!({"status": "ok", "project": name, "root": root.to_string_lossy()}));
                } else {
                    println!(
                        "{}",
                        format!("Project '{name}' ready at {}", root.display()).green()
                    );
                }
                Ok(())
            }
            CliCommand::Lint => {
                let Some(projects) = load_projects(path, "No projects found (no cite.toml found)")?
                else {
                    return Ok(());
                };
                let multi = projects.len() > 1;
                let mut has_warnings = false;
                for ctx in &projects {
                    if multi {
                        println!("{}", format!("── {} ──", ctx.manifest.project.name).green());
                    }
                    let outcome = doctor::lint_all(ctx);
                    if cli.json {
                        print_json(&outcome);
                    } else {
                        outcome.emit();
                    }
                    if outcome.has_warnings() {
                        has_warnings = true;
                    }
                }
                if !cli.json && !has_warnings {
                    println!("{}", "Lint complete — no issues found".green());
                }
                Ok(())
            }
            CliCommand::Build { force } => {
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
                        Ok(outcome) => {
                            if cli.json {
                                match &outcome {
                                    compiler::CompileOutcome::UpToDate => {
                                        print_json(&serde_json::json!({"status": "uptodate"}));
                                    }
                                    compiler::CompileOutcome::Complete { stats, artifact } => {
                                        let mut v = serde_json::to_value(stats).unwrap_or_default();
                                        if let Some(obj) = v.as_object_mut() {
                                            obj.insert("status".into(), "complete".into());
                                            obj.insert("artifact".into(), artifact.to_string_lossy().into());
                                        }
                                        print_json(&v);
                                    }
                                }
                            } else {
                                outcome.emit();
                            }
                        }
                        Err(e) => {
                            error!("Build failed: {e}");
                            has_errors = true;
                        }
                    }
                }
                if has_errors {
                    return Err(CiteError::Config(
                        "Build failed in one or more projects".to_string(),
                    ));
                } else if !cli.json {
                    println!("{}", "Build complete".green());
                }
                Ok(())
            }
            CliCommand::Deploy => {
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
                    match deploy::deploy(ctx, cli.dry_run).await {
                        Ok(msg) => {
                            if cli.json {
                                print_json(&serde_json::json!({"status": "ok", "message": msg}));
                            } else {
                                eprintln!("{msg}");
                            }
                        }
                        Err(e) => {
                            if cli.json {
                                print_json(&serde_json::json!({"status": "error", "message": e.to_string()}));
                            } else {
                                warn!("Deploy failed: {e}");
                            }
                            has_errors = true;
                        }
                    }
                }
                if has_errors {
                    return Err(CiteError::Deploy(
                        "Deploy failed in one or more projects".to_string(),
                    ));
                } else if !cli.json {
                    println!("{}", "Deploy complete".green());
                }
                Ok(())
            }
            CliCommand::Status => {
                let Some(projects) = load_projects(path, "No projects found")? else {
                    return Ok(());
                };
                let multi = projects.len() > 1;
                for ctx in &projects {
                    if multi {
                        println!("{}", format!("── {} ──", ctx.manifest.project.name).green());
                    } else if !cli.json {
                        info!("Project Status");
                    }
                    if cli.json {
                        print_json(&serde_json::json!({"project": ctx.manifest.project.name, "root": ctx.root.to_string_lossy(), "podcasts": ctx.metadata.podcasts.len()}));
                    } else {
                        project::print_status(ctx);
                    }
                }
                if !cli.json {
                    println!("{}", "Status complete".green());
                }
                Ok(())
            }
            CliCommand::Doctor => {
                let root = PathBuf::from(path);
                let Some(projects) = load_projects(path, "")? else {
                    if cli.json {
                        print_json(&serde_json::json!({"status": "noproject", "errors": ["No cite.toml found"]}));
                    } else {
                        info!("Running diagnostics");
                        doctor::check_file(&root, "cite.toml", "run 'cite-cli init'");
                        doctor::check_file(&root, "metadata.yml", "");
                    }
                    return Ok(());
                };
                let multi = projects.len() > 1;
                let mut all_outcomes: Vec<serde_json::Value> = Vec::new();
                let mut has_errors = false;
                let mut has_warnings = false;
                for ctx in &projects {
                    if multi {
                        println!("{}", format!("── {} ──", ctx.manifest.project.name).green());
                    }
                    let outcome = doctor::run(ctx)?;
                    if cli.json
                        && let Ok(v) = serde_json::to_value(&outcome) {
                            all_outcomes.push(v);
                    }
                    if outcome.has_errors() {
                        has_errors = true;
                    }
                    if outcome.has_warnings() {
                        has_warnings = true;
                    }
                }
                if cli.json {
                    print_json(&all_outcomes);
                }
                if has_errors {
                    return Err(CiteError::Config(
                        "Doctor found validation errors".to_string(),
                    ));
                }
                if !cli.json && !has_warnings {
                    println!("{}", "Doctor check complete — no issues found".green());
                }
                Ok(())
            }
            CliCommand::Clean => {
                let Some(projects) = load_projects(path, "No projects found")? else {
                    return Ok(());
                };
                let multi = projects.len() > 1;
                for ctx in &projects {
                    if multi {
                        println!("{}", format!("── {} ──", ctx.manifest.project.name).green());
                    }
                    ctx.clean()?;
                    if cli.json {
                        print_json(&serde_json::json!({"status": "ok", "project": ctx.manifest.project.name}));
                    } else {
                        println!("{}", "Cleaned build artifacts".green());
                    }
                }
                Ok(())
            }
            CliCommand::Rollback { id } => {
                let root = PathBuf::from(path);
                let ctx = project::ProjectContext::load(&root)?;
                let msg = deploy::rollback(&ctx, &id).await?;
                if cli.json {
                    print_json(&serde_json::json!({"status": "ok", "message": msg}));
                } else {
                    info!("{msg}");
                }
                Ok(())
            }
            CliCommand::Login { email, password } => {
                let root = PathBuf::from(path);
                let ctx = project::ProjectContext::load(&root)?;
                deploy::login(&ctx, email, password).await?;
                println!("{}", "Login complete".green());
                Ok(())
            }
            CliCommand::Upgrade => {
                let msg = upgrade::upgrade().await?;
                info!("{msg}");
                println!("{}", "Upgrade complete".green());
                Ok(())
            }
            CliCommand::Uninstall { force } => uninstall::uninstall(force),
        }
    }
}
