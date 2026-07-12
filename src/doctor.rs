use crate::project::ProjectContext;
use crate::report::{CiteError, Style, styled};
use std::path::Path;
use tracing::{error, info, instrument, warn};

pub enum DoctorOutcome {
    Clean,
    Findings {
        errors: Vec<String>,
        warnings: Vec<String>,
    },
}

impl DoctorOutcome {
    pub fn has_errors(&self) -> bool {
        match self {
            DoctorOutcome::Findings { errors, .. } => !errors.is_empty(),
            _ => false,
        }
    }

    pub fn has_warnings(&self) -> bool {
        match self {
            DoctorOutcome::Findings { warnings, .. } => !warnings.is_empty(),
            _ => false,
        }
    }

    pub fn merge(&mut self, other: DoctorOutcome) {
        match other {
            DoctorOutcome::Clean => {}
            DoctorOutcome::Findings {
                errors: new_errors,
                warnings: new_warnings,
            } => {
                for e in new_errors {
                    self.push_error(e);
                }
                for w in new_warnings {
                    self.push_warning(w);
                }
            }
        }
    }

    fn push_error(&mut self, msg: String) {
        match self {
            DoctorOutcome::Findings { errors, .. } => errors.push(msg),
            DoctorOutcome::Clean => {
                *self = DoctorOutcome::Findings {
                    errors: vec![msg],
                    warnings: Vec::new(),
                }
            }
        }
    }

    fn push_warning(&mut self, msg: String) {
        match self {
            DoctorOutcome::Findings { warnings, .. } => warnings.push(msg),
            DoctorOutcome::Clean => {
                *self = DoctorOutcome::Findings {
                    errors: Vec::new(),
                    warnings: vec![msg],
                }
            }
        }
    }

    pub fn print(&self) {
        match self {
            DoctorOutcome::Clean => {}
            DoctorOutcome::Findings { errors, warnings } => {
                for e in errors {
                    error!("{}", e);
                }
                for w in warnings {
                    warn!("{}", w);
                }
                eprintln!(
                    "{} {}",
                    styled(format!("{} error(s)", errors.len()), Style::Error),
                    styled(format!("{} warning(s)", warnings.len()), Style::Warning)
                );
            }
        }
    }
}

fn collect_findings(errors: Vec<String>, warnings: Vec<String>) -> DoctorOutcome {
    if errors.is_empty() && warnings.is_empty() {
        DoctorOutcome::Clean
    } else {
        DoctorOutcome::Findings { errors, warnings }
    }
}

#[instrument(skip(ctx), fields(project = %ctx.manifest.project.name))]
pub fn validate_all(ctx: &ProjectContext) -> DoctorOutcome {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    validate_project_structure(ctx, &mut errors, &mut warnings);
    validate_file_existence(ctx, &mut errors, &mut warnings);
    validate_asset_formats(ctx, &mut warnings);

    collect_findings(errors, warnings)
}

fn validate_project_structure(
    ctx: &ProjectContext,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let required = [
        ("cite.toml", ctx.root.join("cite.toml")),
        (
            &ctx.manifest.project.metadata_file,
            ctx.root.join(&ctx.manifest.project.metadata_file),
        ),
    ];
    for (name, path) in &required {
        if !path.exists() {
            errors.push(format!(
                "Required file '{name}' not found at {}",
                path.display()
            ));
        }
    }

    let dirs = [
        ("content", ctx.content_dir()),
        ("assets/image", ctx.root.join("assets/image")),
        ("assets/audio", ctx.root.join("assets/audio")),
    ];
    for (name, path) in &dirs {
        if !path.is_dir() {
            warnings.push(format!(
                "Directory '{name}' does not exist at {}",
                path.display()
            ));
        }
    }
}

fn validate_file_existence(
    ctx: &ProjectContext,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    for pod in &ctx.metadata.podcasts {
        let path = ctx.root.join(&pod.file);
        if !path.exists() {
            errors.push(format!(
                "Podcast '{}' references file '{}' which does not exist",
                pod.title, pod.file
            ));
        }

        if let Some(cit) = &pod.citation {
            let cit_path = ctx.root.join(cit);
            if !cit_path.exists() {
                warnings.push(format!(
                    "Podcast '{}' references citation file '{}' which does not exist",
                    pod.title, cit
                ));
            }
        }

        if let Some(audio) = &pod.audio {
            let audio_path = ctx.root.join(audio);
            if !audio_path.exists() {
                errors.push(format!(
                    "Podcast '{}' references audio file '{}' which does not exist",
                    pod.title, audio
                ));
            }
        }
    }
}

fn validate_asset_formats(ctx: &ProjectContext, warnings: &mut Vec<String>) {
    let allowed_audio: std::collections::HashSet<&str> = ctx
        .manifest
        .assets
        .audio_formats
        .iter()
        .map(|s| s.as_str())
        .collect();

    for pod in &ctx.metadata.podcasts {
        if let Some(audio) = &pod.audio {
            let ext = Path::new(audio)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if !allowed_audio.contains(ext) {
                warnings.push(format!(
                    "Podcast '{}' has audio file '{audio}' with extension '{ext}' not in allowed audio formats {:?}",
                    pod.title, ctx.manifest.assets.audio_formats
                ));
            }
        }

        let ext = Path::new(&pod.file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !matches!(ext, "md" | "rst") {
            warnings.push(format!(
                "Podcast '{}' has file '{}' with unexpected extension '{ext}'",
                pod.title, pod.file
            ));
        }
    }
}

#[instrument(skip(ctx), fields(project = %ctx.manifest.project.name))]
pub fn lint_all(ctx: &ProjectContext) -> DoctorOutcome {
    let mut warnings = Vec::new();

    for pod in &ctx.metadata.podcasts {
        let path = ctx.root.join(&pod.file);
        if path.exists()
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            let word_count = content.split_whitespace().count();
            if word_count < 10 {
                warnings.push(format!(
                    "Podcast '{}' is very short ({} words)",
                    pod.title, word_count
                ));
            }
        }
    }

    collect_findings(Vec::new(), warnings)
}

pub fn check_file(root: &Path, filename: &str, hint: &str) {
    let path = root.join(filename);
    if path.exists() {
        info!("{filename} found");
    } else if hint.is_empty() {
        warn!("{filename} not found");
    } else {
        warn!("{filename} not found - {hint}");
    }
}

pub fn run(ctx: &ProjectContext) -> Result<DoctorOutcome, CiteError> {
    info!("Running diagnostics");

    let mut outcome = DoctorOutcome::Clean;
    outcome.merge(validate_all(ctx));
    outcome.merge(lint_all(ctx));

    let meta_file = &ctx.manifest.project.metadata_file;
    check_file(&ctx.root, "cite.toml", "run 'cite-cli init'");
    check_file(&ctx.root, meta_file, "");
    for dir in &["content", "assets/audio", "assets/image", "build"] {
        let d = ctx.root.join(dir);
        if d.is_dir() {
            info!("{dir}/ exists");
        } else if *dir == "build" {
            info!("{dir}/ missing (created by build)");
        } else {
            info!("{dir}/ missing (will be created on init)");
        }
    }

    if ctx
        .manifest
        .backend
        .as_ref()
        .and_then(|b| b.staging_url.as_deref())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        info!("Backend configured for staging");
    } else {
        let msg = "No backend configured (deploy will fail)".to_string();
        warn!("{msg}");
        outcome.push_warning(msg);
    }
    if ctx.manifest.build.incremental {
        info!("Incremental builds enabled");
    }
    if ctx.manifest.project.artist_id.is_empty() {
        let msg = "Artist ID is empty - set it in [project] in cite.toml".to_string();
        warn!("{msg}");
        outcome.push_warning(msg);
    } else {
        info!("Artist ID: {}", ctx.manifest.project.artist_id);
    }
    if ctx
        .manifest
        .backend
        .as_ref()
        .and_then(|b| b.staging_service_key.as_deref())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        info!("Using inline staging_service_key from cite.toml");
    } else {
        let msg = "No staging service key found - deploy will fail".to_string();
        warn!("{msg}");
        outcome.push_warning(msg);
    }

    Ok(outcome)
}
