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
        self.metadata
            .content_files()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use std::path::PathBuf;

    #[test]
    fn test_content_files_collects_all_references() {
        let ctx = ProjectContext {
            root: PathBuf::from("/root"),
            manifest: Manifest::default_template("test"),
            metadata: Metadata {
                podcasts: vec![crate::metadata::Podcast {
                    id: "abc".into(),
                    title: "P".into(),
                    file: "content/p.md".into(),
                    source_url: None,
                    category: None,
                    thumbnail: None,
                    audio: Some("assets/audio/p.mp3".into()),
                    citation: Some("content/p.bib".into()),
                    content: None,
                }],
            },
        };

        let files = ctx.content_files();
        assert_eq!(files.len(), 3);
        assert!(files.contains(&PathBuf::from("/root/content/p.md")));
        assert!(files.contains(&PathBuf::from("/root/content/p.bib")));
        assert!(files.contains(&PathBuf::from("/root/assets/audio/p.mp3")));
    }
}
