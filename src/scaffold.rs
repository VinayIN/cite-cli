use crate::error::CiteError;
use crate::manifest::Manifest;
use std::path::Path;
use tracing::instrument;

#[instrument(skip(root), fields(project = %name))]
pub fn init_project(name: &str, root: &Path) -> Result<(), CiteError> {
    if root.exists() && root.read_dir()?.next().is_some() {
        return Err(CiteError::Config(format!(
            "Directory '{}' already exists and is not empty",
            root.display()
        )));
    }

    // Create directory structure
    let dirs = [
        root.join("content"),
        root.join("assets/audio"),
        root.join("assets/images"),
        root.join("build"),
    ];
    for dir in &dirs {
        std::fs::create_dir_all(dir)?;
    }

    // Write cite.toml
    let manifest = Manifest::default_template(name);
    let toml_str = toml::to_string_pretty(&manifest)
        .map_err(|e| CiteError::Config(format!("Failed to serialize manifest: {e}")))?;
    std::fs::write(root.join("cite.toml"), toml_str)?;

    // Write starter metadata.yml
    let metadata_yaml = r#"# cite-cli metadata
# Add your content entries below.

artists: []
news: []
podcasts: []
newsletters: []
timelines: []
"#;
    std::fs::write(root.join("metadata.yml"), metadata_yaml)?;

    // Write .gitignore
    let gitignore = "build/\n.cite-cache.json\n";
    std::fs::write(root.join(".gitignore"), gitignore)?;

    Ok(())
}

#[instrument(skip(root))]
pub fn clean_project(root: &Path) -> Result<(), CiteError> {
    let build_dir = root.join("build");
    if build_dir.exists() {
        std::fs::remove_dir_all(&build_dir)?;
    }
    let cache = root.join(".cite-cache.json");
    if cache.exists() {
        std::fs::remove_file(&cache)?;
    }
    Ok(())
}
