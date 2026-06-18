use crate::error::CiteError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use tokio::io::AsyncReadExt;
use tracing::instrument;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildCache {
    pub compiler_version: String,
    pub hashes: HashMap<String, String>,
}

impl BuildCache {
    pub fn new(compiler_version: &str, hashes: HashMap<String, String>) -> Self {
        Self {
            compiler_version: compiler_version.to_string(),
            hashes,
        }
    }

    pub async fn load_or_default(path: &Path) -> Result<Self, CiteError> {
        if path.exists() {
            let data = tokio::fs::read_to_string(path).await?;
            Ok(serde_json::from_str(&data).unwrap_or_else(|_| BuildCache {
                compiler_version: String::new(),
                hashes: HashMap::new(),
            }))
        } else {
            Ok(BuildCache {
                compiler_version: String::new(),
                hashes: HashMap::new(),
            })
        }
    }

    pub async fn save(&self, path: &Path) -> Result<(), CiteError> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let data = serde_json::to_string_pretty(self)?;
        tokio::fs::write(path, data).await?;
        Ok(())
    }

    pub fn changed_since(&self, current: &HashMap<String, String>) -> Vec<String> {
        let mut changed = Vec::new();
        for (path, hash) in current {
            match self.hashes.get(path) {
                Some(old) if old == hash => {}
                _ => changed.push(path.clone()),
            }
        }
        for path in self.hashes.keys() {
            if !current.contains_key(path) {
                changed.push(path.clone());
            }
        }
        changed
    }
}

#[instrument(skip(files))]
pub async fn hash_files(files: &[impl AsRef<Path>]) -> Result<HashMap<String, String>, CiteError> {
    let mut hashes = HashMap::new();
    for file in files {
        let path = file.as_ref();
        if path.exists() {
            let mut f = tokio::fs::File::open(path).await?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).await?;
            let hash = Sha256::digest(&buf)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            hashes.insert(path.to_string_lossy().to_string(), hash);
        }
    }
    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_changed_since_new_file() {
        let cache = BuildCache::new("0", HashMap::new());
        let mut current = HashMap::new();
        current.insert("a.md".into(), "abc".into());
        let changed = cache.changed_since(&current);
        assert_eq!(changed, vec!["a.md"]);
    }

    #[test]
    fn test_changed_since_unchanged() {
        let mut hashes = HashMap::new();
        hashes.insert("a.md".into(), "abc".into());
        let cache = BuildCache::new("0", hashes);
        let mut current = HashMap::new();
        current.insert("a.md".into(), "abc".into());
        let changed = cache.changed_since(&current);
        assert!(changed.is_empty());
    }

    #[test]
    fn test_changed_since_modified() {
        let mut hashes = HashMap::new();
        hashes.insert("a.md".into(), "abc".into());
        let cache = BuildCache::new("0", hashes);
        let mut current = HashMap::new();
        current.insert("a.md".into(), "def".into());
        let changed = cache.changed_since(&current);
        assert_eq!(changed, vec!["a.md"]);
    }
}
