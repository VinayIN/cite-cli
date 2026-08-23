use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::core::CiteError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupabaseCredentials {
    pub url: String,
    pub api_key: String,
}

fn creds_path() -> PathBuf {
    if let Ok(path) = std::env::var("CITE_CREDS_PATH") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".cite").join("credentials.toml")
}

pub fn load_credentials() -> Result<SupabaseCredentials, CiteError> {
    if let (Ok(url), Ok(key)) = (
        std::env::var("CITE_SUPABASE_URL"),
        std::env::var("CITE_SUPABASE_API_KEY"),
    ) {
        info!("Using credentials from environment variables");
        return Ok(SupabaseCredentials { url, api_key: key });
    }

    let path = creds_path();
    if !path.exists() {
        return Err(CiteError::Config(
            "No credentials found. Run 'cite-cli login' or set CITE_SUPABASE_URL and CITE_SUPABASE_API_KEY environment variables."
                .to_string(),
        ));
    }

    let s = std::fs::read_to_string(&path)?;
    let table: toml::Value = toml::from_str(&s)?;
    let supabase = table.get("supabase").ok_or_else(|| {
        CiteError::Config("Missing [supabase] section in credentials.toml".into())
    })?;
    let url = supabase
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CiteError::Config("Missing url in [supabase]".into()))?
        .to_string();
    let api_key = supabase
        .get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| CiteError::Config("Missing api_key in [supabase]".into()))?
        .to_string();

    info!("Loaded credentials from {}", path.display());
    Ok(SupabaseCredentials { url, api_key })
}

pub fn save_credentials(creds: &SupabaseCredentials) -> Result<(), CiteError> {
    let path = creds_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = format!(
        r#"[supabase]
url = "{}"
api_key = "{}"
"#,
        creds.url, creds.api_key
    );
    std::fs::write(&path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    struct EnvGuard {
        key: String,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let previous = env::var(key).ok();
            unsafe {
                env::set_var(key, value);
            }
            Self {
                key: key.to_string(),
                previous,
            }
        }

        fn remove(key: &str) -> Self {
            let previous = env::var(key).ok();
            unsafe {
                env::remove_var(key);
            }
            Self {
                key: key.to_string(),
                previous,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(val) => unsafe {
                    env::set_var(&self.key, val);
                },
                None => unsafe {
                    env::remove_var(&self.key);
                },
            }
        }
    }

    #[test]
    fn test_creds_path_default() {
        let p = creds_path();
        assert!(p.to_string_lossy().contains(".cite/credentials.toml"));
    }

    #[test]
    fn test_credentials_from_env() {
        let _guard_url = EnvGuard::set("CITE_SUPABASE_URL", "https://env-test.supabase.co");
        let _guard_key = EnvGuard::set("CITE_SUPABASE_API_KEY", "env-key-456");

        let result = load_credentials();
        assert!(result.is_ok());
        let creds = result.unwrap();
        assert_eq!(creds.url, "https://env-test.supabase.co");
        assert_eq!(creds.api_key, "env-key-456");
    }

    #[test]
    fn test_credentials_file_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let creds_file = dir.path().join("credentials.toml");
        std::fs::write(&creds_file, "not-toml").unwrap();

        let _guard = EnvGuard::set("CITE_CREDS_PATH", creds_file.to_str().unwrap());
        let result = load_credentials();
        assert!(result.is_err());
    }

    #[test]
    fn test_credentials_missing_supabase_section() {
        let dir = tempfile::tempdir().unwrap();
        let creds_file = dir.path().join("credentials.toml");
        std::fs::write(&creds_file, "[other]\nkey = \"val\"").unwrap();

        let _guard = EnvGuard::set("CITE_CREDS_PATH", creds_file.to_str().unwrap());
        let _url_guard = EnvGuard::remove("CITE_SUPABASE_URL");
        let _key_guard = EnvGuard::remove("CITE_SUPABASE_API_KEY");
        let result = load_credentials();
        assert!(result.is_err());
        match result {
            Err(CiteError::Config(msg)) => assert!(msg.contains("[supabase]"), "{msg}"),
            _ => panic!("expected Config error"),
        }
    }
}
