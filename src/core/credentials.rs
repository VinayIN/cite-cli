use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::core::CiteError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupabaseCredentials {
    pub url: String,
    pub api_key: String,
}

pub fn creds_path() -> PathBuf {
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
    let supabase = table
        .get("supabase")
        .ok_or_else(|| CiteError::Config("Missing [supabase] section in credentials.toml".into()))?;
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

    #[test]
    fn test_creds_path_default() {
        let p = creds_path();
        assert!(p.to_string_lossy().contains(".cite/credentials.toml"));
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".cite");
        std::fs::create_dir_all(&path).unwrap();
        let creds_file = path.join("credentials.toml");

        let saved = SupabaseCredentials {
            url: "https://test.supabase.co".into(),
            api_key: "test-key-123".into(),
        };
        let content = format!(
            "[supabase]\nurl = \"{}\"\napi_key = \"{}\"\n",
            saved.url, saved.api_key
        );
        std::fs::write(&creds_file, &content).unwrap();

        let s = std::fs::read_to_string(&creds_file).unwrap();
        let table: toml::Value = toml::from_str(&s).unwrap();
        let supabase = table.get("supabase").unwrap();
        let loaded = SupabaseCredentials {
            url: supabase.get("url").and_then(|v| v.as_str()).unwrap().to_string(),
            api_key: supabase.get("api_key").and_then(|v| v.as_str()).unwrap().to_string(),
        };
        assert_eq!(loaded.url, "https://test.supabase.co");
        assert_eq!(loaded.api_key, "test-key-123");
    }

    #[test]
    fn test_credentials_from_env() {
        let prev_url = env::var("CITE_SUPABASE_URL").ok();
        let prev_key = env::var("CITE_SUPABASE_API_KEY").ok();
        unsafe {
            env::set_var("CITE_SUPABASE_URL", "https://env-test.supabase.co");
            env::set_var("CITE_SUPABASE_API_KEY", "env-key-456");
        }

        let result = load_credentials();
        assert!(result.is_ok());
        let creds = result.unwrap();
        assert_eq!(creds.url, "https://env-test.supabase.co");
        assert_eq!(creds.api_key, "env-key-456");

        unsafe {
            match prev_url {
                Some(u) => env::set_var("CITE_SUPABASE_URL", u),
                None => env::remove_var("CITE_SUPABASE_URL"),
            }
            match prev_key {
                Some(k) => env::set_var("CITE_SUPABASE_API_KEY", k),
                None => env::remove_var("CITE_SUPABASE_API_KEY"),
            }
        }
    }

    #[test]
    fn test_credentials_file_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".cite");
        std::fs::create_dir_all(&path).unwrap();
        let creds_file = path.join("credentials.toml");
        std::fs::write(&creds_file, "not-toml").unwrap();

        let s = std::fs::read_to_string(&creds_file).unwrap();
        let result: Result<toml::Value, _> = toml::from_str(&s);
        assert!(result.is_err());
    }

    #[test]
    fn test_credentials_missing_supabase_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".cite");
        std::fs::create_dir_all(&path).unwrap();
        let creds_file = path.join("credentials.toml");
        std::fs::write(&creds_file, "[other]\nkey = \"val\"").unwrap();

        let s = std::fs::read_to_string(&creds_file).unwrap();
        let table: toml::Value = toml::from_str(&s).unwrap();
        let supabase = table.get("supabase");
        assert!(supabase.is_none());
    }
}
