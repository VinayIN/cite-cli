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

    pub fn load_or_default(path: &Path) -> Result<Self, CiteError> {
        if path.exists() {
            let data = std::fs::read_to_string(path)?;
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

    pub fn save(&self, path: &Path) -> Result<(), CiteError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
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
