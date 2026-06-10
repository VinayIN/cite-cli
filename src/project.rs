use crate::error::CiteError;
use crate::manifest::Manifest;
use crate::metadata::Metadata;
use std::path::{Path, PathBuf};
use tracing::instrument;

#[derive(Debug, Clone)]
pub struct ProjectContext {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub metadata: Metadata,
}

impl ProjectContext {
    #[instrument(skip(root), fields(path = %root.display()))]
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
        let mut files = Vec::new();
        for news_item in &self.metadata.news {
            files.push(self.root.join(&news_item.file));
            if let Some(cit) = &news_item.citation {
                files.push(self.root.join(cit));
            }
        }
        for pod in &self.metadata.podcasts {
            files.push(self.root.join(&pod.file));
        }
        for nl in &self.metadata.newsletters {
            if let Some(f) = &nl.file {
                files.push(self.root.join(f));
            }
        }
        files
    }
}

impl Metadata {
    pub fn all_slugs(&self) -> Vec<(&'static str, &crate::slug::Slug)> {
        let mut slugs = Vec::new();
        for a in &self.artists {
            slugs.push(("artists", &a.slug));
        }
        for n in &self.news {
            slugs.push(("news", &n.slug));
        }
        for p in &self.podcasts {
            slugs.push(("podcasts", &p.slug));
        }
        for n in &self.newsletters {
            slugs.push(("newsletters", &n.slug));
        }
        for t in &self.timelines {
            slugs.push(("timelines", &t.slug));
        }
        slugs
    }
}
