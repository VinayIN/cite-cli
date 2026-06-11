use crate::error::CiteError;
use crate::manifest::Manifest;
use std::fs;
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

    for sub in ["content", "assets/audio", "assets/images", "build"] {
        fs::create_dir_all(root.join(sub))?;
    }

    let manifest = Manifest::default_template(name);
    let toml_str =
        toml::to_string_pretty(&manifest).map_err(|e| CiteError::Config(e.to_string()))?;
    fs::write(root.join("cite.toml"), toml_str)?;

    let metadata_yaml = r"
# cite-cli metadata
# Add your content entries below.

artists: []
news: []
podcasts: []
newsletters: []
";
    fs::write(root.join("metadata.yml"), metadata_yaml)?;

    fs::write(root.join(".gitignore"), "build/\n.cite-cache.json\n")?;

    Ok(())
}

#[instrument(skip(root))]
pub fn clean_project(root: &Path) -> Result<(), CiteError> {
    let build_dir = root.join("build");
    if build_dir.exists() {
        fs::remove_dir_all(&build_dir)?;
    }
    let cache = root.join(".cite-cache.json");
    if cache.exists() {
        fs::remove_file(&cache)?;
    }
    Ok(())
}
