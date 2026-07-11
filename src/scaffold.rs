use crate::error::CiteError;
use crate::manifest::Manifest;
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
        report.directories_created.push(root.to_string_lossy().to_string());
    }

    for sub in ["content", "assets/audio", "assets/images", "build"] {
        let path = root.join(sub);
        if !path.exists() {
            fs::create_dir_all(&path)?;
            report.directories_created.push(sub.to_string());
        }
    }

    let manifest_path = root.join("cite.toml");
    if !manifest_path.exists() {
        let manifest = Manifest::default_template(name);
        let toml_str =
            toml::to_string_pretty(&manifest).map_err(|e| CiteError::Config(e.to_string()))?;
        fs::write(&manifest_path, toml_str)?;
        report.files_created.push("cite.toml".to_string());
    } else {
        report.files_skipped.push("cite.toml (already exists)".to_string());
    }

    let meta_path = root.join("metadata.yml");
    if !meta_path.exists() {
        let metadata_yaml = "podcasts: []\n";
        fs::write(&meta_path, metadata_yaml)?;
        report.files_created.push("metadata.yml".to_string());
    } else {
        report.files_skipped.push("metadata.yml (already exists)".to_string());
    }

    let gitignore_path = root.join(".gitignore");
    if !gitignore_path.exists() {
        fs::write(&gitignore_path, "build/\n.cite-cache.json\n")?;
        report.files_created.push(".gitignore".to_string());
    } else {
        report.files_skipped.push(".gitignore (already exists)".to_string());
    }

    Ok(report)
}


