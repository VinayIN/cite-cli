use crate::error::CiteError;
use crate::manifest::Manifest;
use crate::metadata::Metadata;
use std::fs;
use std::path::Path;
use tracing::instrument;

pub struct InitReport {
    pub directories_created: Vec<String>,
    pub files_created: Vec<String>,
    pub files_skipped: Vec<String>,
}

#[instrument(skip(root), fields(project = %name))]
pub fn init_project(name: &str, root: &Path) -> Result<InitReport, CiteError> {
    let mut report = InitReport {
        directories_created: vec![],
        files_created: vec![],
        files_skipped: vec![],
    };

    if !root.exists() {
        fs::create_dir_all(root)?;
        report
            .directories_created
            .push(root.to_string_lossy().to_string());
    }

    for sub in ["content", "assets/audio", "assets/images"] {
        let path = root.join(sub);
        if !path.exists() {
            fs::create_dir_all(&path)?;
            report.directories_created.push(sub.to_string());
        }
    }

    let manifest_path = root.join("cite.toml");
    if !manifest_path.exists() {
        fs::write(&manifest_path, manifest_template(name)?)?;
        report.files_created.push("cite.toml".to_string());
    } else {
        report
            .files_skipped
            .push("cite.toml (already exists)".to_string());
    }

    let meta_path = root.join("metadata.yml");
    if !meta_path.exists() {
        let metadata_yaml = Metadata::default_template();
        fs::write(&meta_path, serde_yaml::to_string(&metadata_yaml)?)?;
        report.files_created.push("metadata.yml".to_string());
    } else {
        report
            .files_skipped
            .push("metadata.yml (already exists)".to_string());
    }

    let gitignore_path = root.join(".gitignore");
    if !gitignore_path.exists() {
        fs::write(&gitignore_path, "build/\n.cite-cache.json\n")?;
        report.files_created.push(".gitignore".to_string());
    } else {
        report
            .files_skipped
            .push(".gitignore (already exists)".to_string());
    }

    Ok(report)
}

fn manifest_template(name: &str) -> Result<String, CiteError> {
    let manifest = Manifest::default_template(name);
    let enabled_extensions = manifest
        .compiler
        .enabled_extensions
        .iter()
        .map(|ext| format!("\"{ext}\""))
        .collect::<Vec<_>>()
        .join(", ");

    Ok(format!(
        r#"
# cite-cli project manifest
[project]
# Project name.
name = "{name}"
# language for generated content.
language = "{language}"
# Metadata file loaded by the CLI.
metadata_file = "{metadata_file}"
# Artist UUID from the database.
artist_id = "{artist_id}"

[build]
# Compiler protocol version.
compiler_version = {compiler_version}
# Rebuild only when inputs change.
incremental = {incremental}
# Build artifact format.
output_format = "{output_format}"

# Optional deployment settings.
# [backend]
# Staging URL, for example: https://your-project.supabase.co
# staging_url = ""
# Service key for staging deploys.
# staging_service_key = ""

[compiler]
# Enabled compiler extensions.
enabled_extensions = [{enabled_extensions}]

[assets]
# Allowed audio formats.
audio_formats = [{audio_formats}]
# Allowed image formats.
image_formats = [{image_formats}]

[validation]
# Enable strict validation rules.
strict = {strict}
"#,
        name = manifest.project.name,
        language = manifest.project.language,
        metadata_file = manifest.project.metadata_file,
        artist_id = manifest.project.artist_id,
        compiler_version = manifest.build.compiler_version,
        incremental = manifest.build.incremental,
        output_format = manifest.build.output_format,
        enabled_extensions = enabled_extensions,
        audio_formats = manifest
            .assets
            .audio_formats
            .iter()
            .map(|ext| format!("\"{ext}\""))
            .collect::<Vec<_>>()
            .join(", "),
        image_formats = manifest
            .assets
            .image_formats
            .iter()
            .map(|ext| format!("\"{ext}\""))
            .collect::<Vec<_>>()
            .join(", "),
        strict = manifest.validation.strict,
    ))
}
