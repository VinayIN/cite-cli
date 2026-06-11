use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "cite-cli",
    version,
    about = "Manage news content projects from scaffolding through deployment"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Enable verbose output (trace logging)
    #[arg(global = true, short, long)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a new project with recommended structure and starter files
    Init {
        /// Project name
        name: String,
        /// Target directory (defaults to <name> in current dir)
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Run full validation (structure, files, metadata, cross-references)
    Validate {
        /// Project directory path
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Run linting rules (naming, style, word counts)
    Lint {
        /// Project directory path
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Execute the compiler protocol and produce a build artifact
    Build {
        /// Project directory path
        #[arg(short, long)]
        path: Option<String>,
        /// Force full rebuild, ignoring cache
        #[arg(long)]
        force: bool,
    },
    /// Deploy the built project to Supabase staging
    Deploy {
        /// Project directory path
        #[arg(short, long)]
        path: Option<String>,
        /// Dry run mode - no data sent
        #[arg(long)]
        dry_run: bool,
    },
    /// Show project health, validation summary, and sync state
    Status {
        /// Project directory path
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Diagnose common project issues and configuration problems
    Doctor {
        /// Project directory path
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Remove build artifacts, temporary files, and build cache
    Clean {
        /// Project directory path
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Rollback a deployment by its unique ID
    Rollback {
        /// Deployment ID to rollback
        id: String,
        /// Project directory path
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Self-update to the latest GitHub release
    Upgrade,
    /// Remove cite-cli binary and clean up shell configuration
    Uninstall {
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },
}
