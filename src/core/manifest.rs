use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    pub name: String,
    pub language: String,
    pub metadata_file: String,
    pub artist_id: String,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            language: "en".to_string(),
            metadata_file: "metadata.yml".to_string(),
            artist_id: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildConfig {
    pub compiler_version: f64,
    pub incremental: bool,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            compiler_version: 1.0,
            incremental: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct BackendConfig {
    pub staging_url: Option<String>,
    pub staging_service_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Manifest {
    pub project: ProjectConfig,
    pub build: BuildConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<BackendConfig>,
}

impl Manifest {
    pub fn default_template(name: &str) -> Self {
        Self {
            project: ProjectConfig {
                name: name.to_string(),
                ..Default::default()
            },
            build: BuildConfig::default(),
            backend: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_template() {
        let m = Manifest::default_template("my-project");
        assert_eq!(m.project.name, "my-project");
        assert_eq!(m.project.language, "en");
        assert_eq!(m.project.metadata_file, "metadata.yml");
        assert_eq!(m.build.compiler_version, 1.0);
        assert!(m.build.incremental);
        assert!(m.backend.is_none());
        assert!(m.project.artist_id.is_empty());
    }

    #[test]
    fn test_deserialize_full() {
        let toml_str = r#"
[project]
name = "test"
language = "en"
metadata_file = "example.yml"
artist_id = "11111111-1111-1111-1111-111111111111"

[build]
compiler_version = 1.0
incremental = false

[backend]
staging_url = "https://example.com"
staging_service_key = ""

[compiler]
enabled_extensions = ["tables"]

[assets]
audio_formats = ["mp3"]
image_formats = ["jpg"]

[validation]
strict = true
"#;
        let m: Manifest = toml::from_str(toml_str).unwrap();
        assert_eq!(m.project.name, "test");
        assert_eq!(m.project.language, "en");
        assert_eq!(m.project.metadata_file, "example.yml");
        assert_eq!(m.project.artist_id, "11111111-1111-1111-1111-111111111111");
        assert_eq!(m.build.compiler_version, 1.0);
        assert!(!m.build.incremental);
        assert_eq!(
            m.backend.as_ref().unwrap().staging_url.as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn test_deserialize_partial_applies_defaults() {
        let toml_str = r#"
[project]
name = "partial"
artist_id = "abc"
"#;
        let m: Manifest = toml::from_str(toml_str).unwrap();
        assert_eq!(m.project.name, "partial");
        assert_eq!(m.project.language, "en");
        assert_eq!(m.project.metadata_file, "metadata.yml");
        assert_eq!(m.project.artist_id, "abc");
        assert_eq!(m.build.compiler_version, 1.0);
        assert!(m.build.incremental);
    }
}
