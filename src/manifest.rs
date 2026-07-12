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
    pub output_format: String,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            compiler_version: 1.0,
            incremental: true,
            output_format: "json".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BackendConfig {
    pub staging_url: Option<String>,
    pub staging_service_key: Option<String>,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            staging_url: None,
            staging_service_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompilerConfig {
    pub enabled_extensions: Vec<String>,
}

impl Default for CompilerConfig {
    fn default() -> Self {
        Self {
            enabled_extensions: vec!["tables".to_string(), "footnotes".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AssetsConfig {
    pub audio_formats: Vec<String>,
    pub image_formats: Vec<String>,
}

impl Default for AssetsConfig {
    fn default() -> Self {
        Self {
            audio_formats: vec!["mp3".to_string(), "wav".to_string(), "m4a".to_string()],
            image_formats: vec!["jpg".to_string(), "png".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ValidationConfig {
    pub strict: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self { strict: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub project: ProjectConfig,
    pub build: BuildConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<BackendConfig>,
    pub compiler: CompilerConfig,
    pub assets: AssetsConfig,
    pub validation: ValidationConfig,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            project: ProjectConfig::default(),
            build: BuildConfig::default(),
            backend: None,
            compiler: CompilerConfig::default(),
            assets: AssetsConfig::default(),
            validation: ValidationConfig::default(),
        }
    }
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
            compiler: CompilerConfig::default(),
            assets: AssetsConfig::default(),
            validation: ValidationConfig::default(),
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
        assert!(m.validation.strict);
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
output_format = "json"

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
        assert_eq!(m.build.output_format, "json");
        assert!(!m.build.incremental);
        assert_eq!(m.compiler.enabled_extensions, vec!["tables"]);
        assert_eq!(m.assets.audio_formats, vec!["mp3"]);
        assert_eq!(m.assets.image_formats, vec!["jpg"]);
        assert!(m.validation.strict);
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
        assert_eq!(m.build.output_format, "json");
        assert_eq!(m.compiler.enabled_extensions, vec!["tables", "footnotes"]);
        assert_eq!(m.assets.audio_formats, vec!["mp3", "wav", "m4a"]);
        assert_eq!(m.assets.image_formats, vec!["jpg", "png"]);
        assert!(m.validation.strict);
    }
}
