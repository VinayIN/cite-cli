use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::CiteError;

#[derive(Debug, Clone)]
pub struct BuildCache {
    pub compiler_version: f64,
    pub hashes: HashMap<String, String>,
}

impl BuildCache {
    pub fn new(compiler_version: f64, hashes: HashMap<String, String>) -> Self {
        Self {
            compiler_version,
            hashes,
        }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UuidCache {
    pub mapping: HashMap<String, String>,
}

impl UuidCache {
    pub fn load(root: &Path) -> Self {
        let path = root.join(".cite").join("cache").join("uuid_map.json");
        match std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()) {
            Some(m) => m,
            None => Self {
                mapping: HashMap::new(),
            },
        }
    }

    pub fn save(&self, root: &Path) {
        let dir = root.join(".cite").join("cache");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("uuid_map.json");
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn get_or_create(&mut self, key: &str) -> String {
        if let Some(id) = self.mapping.get(key) {
            return id.clone();
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.mapping.insert(key.to_string(), id.clone());
        id
    }
}

pub async fn hash_files(files: &[impl AsRef<Path>]) -> Result<HashMap<String, String>, CiteError> {
    let mut hashes = HashMap::new();
    for file in files {
        let path = file.as_ref();
        if path.exists() && path.is_file() {
            let mut f = tokio::fs::File::open(path).await?;
            let mut buf = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut f, &mut buf).await?;
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
        let cache = BuildCache::new(0.0, HashMap::new());
        let mut current = HashMap::new();
        current.insert("a.md".into(), "abc".into());
        let changed = cache.changed_since(&current);
        assert_eq!(changed, vec!["a.md"]);
    }

    #[test]
    fn test_changed_since_unchanged() {
        let mut hashes = HashMap::new();
        hashes.insert("a.md".into(), "abc".into());
        let cache = BuildCache::new(0.0, hashes);
        let mut current = HashMap::new();
        current.insert("a.md".into(), "abc".into());
        let changed = cache.changed_since(&current);
        assert!(changed.is_empty());
    }

    #[test]
    fn test_changed_since_modified() {
        let mut hashes = HashMap::new();
        hashes.insert("a.md".into(), "abc".into());
        let cache = BuildCache::new(0.0, hashes);
        let mut current = HashMap::new();
        current.insert("a.md".into(), "def".into());
        let changed = cache.changed_since(&current);
        assert_eq!(changed, vec!["a.md"]);
    }

    #[test]
    fn test_uuid_cache_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = UuidCache::load(dir.path());
        let id = cache.get_or_create("test-key");
        assert!(!id.is_empty());
        cache.save(dir.path());
        let loaded = UuidCache::load(dir.path());
        assert_eq!(loaded.mapping.get("test-key"), Some(&id));
    }
}
