use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub project: ProjectConfig,
    #[serde(default)]
    pub build: BuildConfig,
    pub backend: Option<BackendConfig>,
    #[serde(default)]
    pub compiler: CompilerConfig,
    #[serde(default)]
    pub assets: AssetsConfig,
    #[serde(default)]
    pub validation: ValidationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    pub version: String,
    #[serde(default = "default_language")]
    pub default_language: String,
    #[serde(default = "default_metadata_file")]
    pub metadata_file: String,
}

fn default_language() -> String {
    "en".into()
}
fn default_metadata_file() -> String {
    "metadata.yml".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    #[serde(default = "default_compiler_version")]
    pub compiler_version: String,
    #[serde(default = "default_incremental")]
    pub incremental: bool,
    #[serde(default = "default_output_format")]
    pub output_format: String,
}

fn default_compiler_version() -> String {
    "0".into()
}
fn default_incremental() -> bool {
    true
}
fn default_output_format() -> String {
    "json".into()
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            compiler_version: default_compiler_version(),
            incremental: default_incremental(),
            output_format: default_output_format(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    pub staging_url: String,
    #[serde(default)]
    pub staging_service_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilerConfig {
    #[serde(default)]
    pub enabled_extensions: Vec<String>,
}

impl Default for CompilerConfig {
    fn default() -> Self {
        Self {
            enabled_extensions: vec!["tables".into(), "footnotes".into()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetsConfig {
    #[serde(default = "default_audio_formats")]
    pub audio_formats: Vec<String>,
    #[serde(default = "default_image_formats")]
    pub image_formats: Vec<String>,
}

fn default_audio_formats() -> Vec<String> {
    vec!["mp3".into(), "wav".into(), "m4a".into()]
}
fn default_image_formats() -> Vec<String> {
    vec!["jpg".into(), "png".into()]
}

impl Default for AssetsConfig {
    fn default() -> Self {
        Self {
            audio_formats: default_audio_formats(),
            image_formats: default_image_formats(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    #[serde(default = "default_strict")]
    pub strict: bool,
}

fn default_strict() -> bool {
    true
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            strict: default_strict(),
        }
    }
}

impl Manifest {
    pub fn default_template(name: &str) -> Self {
        Self {
            project: ProjectConfig {
                name: name.to_string(),
                version: "0.1.0".to_string(),
                default_language: default_language(),
                metadata_file: default_metadata_file(),
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
        assert_eq!(m.project.version, "0.1.0");
        assert_eq!(m.project.default_language, "en");
        assert_eq!(m.project.metadata_file, "metadata.yml");
        assert_eq!(m.build.compiler_version, "0");
        assert!(m.build.incremental);
        assert!(m.backend.is_none());
        assert!(m.validation.strict);
    }

    #[test]
    fn test_deserialize_full() {
        let toml_str = r#"
[project]
name = "test"
version = "1.0.0"

[build]
compiler_version = "1"
incremental = false

[backend]
staging_url = "https://example.com"

[compiler]
enabled_extensions = ["tables"]

[assets]
audio_formats = ["mp3"]
image_formats = ["jpg"]
"#;
        let m: Manifest = toml::from_str(toml_str).unwrap();
        assert_eq!(m.project.name, "test");
        assert_eq!(m.project.version, "1.0.0");
        assert_eq!(m.build.compiler_version, "1");
        assert!(!m.build.incremental);
        assert!(m.backend.is_some());
        assert_eq!(
            m.backend.as_ref().unwrap().staging_url,
            "https://example.com"
        );
        assert_eq!(m.assets.audio_formats, vec!["mp3"]);
    }
}
