use std::path::{Path, PathBuf};

use crate::core::CiteError;
use crate::core::manifest::Manifest;
use crate::core::metadata::Metadata;
use tracing::info;

#[derive(Debug, Clone)]
pub struct ProjectContext {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub metadata: Metadata,
}

impl ProjectContext {
    pub fn load(root: &Path) -> Result<Self, CiteError> {
        let manifest_path = root.join("cite.toml");
        if !manifest_path.exists() {
            return Err(CiteError::Config(format!(
                "No cite.toml found at '{}'. Run 'cite-cli init' first.",
                manifest_path.display()
            )));
        }
        let toml_str = std::fs::read_to_string(&manifest_path)?;
        let manifest: Manifest = toml::from_str(&toml_str)?;

        let meta_path = root.join(&manifest.project.metadata_file);
        let metadata = if meta_path.exists() {
            let yaml_str = std::fs::read_to_string(&meta_path)?;
            serde_yaml::from_str(&yaml_str)?
        } else {
            Metadata::default()
        };

        Ok(Self {
            root: root.to_path_buf(),
            manifest,
            metadata,
        })
    }

    pub fn content_dir(&self) -> PathBuf {
        self.root.join("content")
    }

    pub fn build_dir(&self) -> PathBuf {
        self.root.join("build")
    }

    pub fn cache_path(&self) -> PathBuf {
        self.root.join(".cite-cache.json")
    }

    pub fn content_files(&self) -> Vec<PathBuf> {
        self.metadata
            .referenced_files()
            .iter()
            .map(|f| self.root.join(f))
            .collect()
    }

    pub fn clean(&self) -> Result<(), CiteError> {
        let build_dir = self.build_dir();
        if build_dir.exists() {
            std::fs::remove_dir_all(&build_dir)?;
        }
        let cache = self.cache_path();
        if cache.exists() {
            std::fs::remove_file(&cache)?;
        }
        Ok(())
    }
}

pub fn print_status(ctx: &ProjectContext) {
    info!("Name: {}", ctx.manifest.project.name);
    info!("Root: {}", ctx.root.display());
    info!("Artist ID: {}", ctx.manifest.project.artist_id);
    if let Some(b) = &ctx.manifest.backend
        && let Some(u) = &b.staging_url
    {
        info!("Staging: {u}");
    }
    info!("Podcasts: {}", ctx.metadata.podcasts.len());
    let build_path = ctx.build_dir().join("content.json");
    if build_path.exists() {
        info!("Build: exists");
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
            info!("Built: {since}");
        }
    } else {
        info!("Build: not built");
    }
}

pub fn discover_projects(root: &Path) -> Vec<PathBuf> {
    let mut projects = Vec::new();

    if root.join("cite.toml").exists() {
        projects.push(root.to_path_buf());
    }

    if let Ok(entries) = std::fs::read_dir(root) {
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
