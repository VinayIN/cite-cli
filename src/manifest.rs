use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub project: ProjectConfig,
    #[serde(default)]
    pub build: BuildConfig,
    #[serde(default)]
    pub backend: Option<BackendConfig>,
    #[serde(default)]
    pub compiler: CompilerConfig,
    #[serde(default)]
    pub assets: AssetsConfig,
    #[serde(default)]
    pub validation: ValidationConfig,
}

fn get_language() -> String {
    "en".to_string()
}

fn get_metadata_file() -> String {
    "metadata.yml".to_string()
}

fn get_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default = "get_language")]
    pub language: String,
    #[serde(default = "get_metadata_file")]
    pub metadata_file: String,
    #[serde(default = "get_uuid")]
    pub artist_id: String,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            language: get_language(),
            metadata_file: get_metadata_file(),
            artist_id: get_uuid(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    #[serde(default = "get_compiler_version")]
    pub compiler_version: f64,
    #[serde(default = "get_incremental")]
    pub incremental: bool,
    #[serde(default = "get_output_format")]
    pub output_format: String,
}

fn get_compiler_version() -> f64 {
    1.0
}

fn get_incremental() -> bool {
    true
}

fn get_output_format() -> String {
    "json".into()
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            compiler_version: get_compiler_version(),
            incremental: get_incremental(),
            output_format: get_output_format(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackendConfig {
    #[serde(default)]
    pub staging_url: String,
    #[serde(default)]
    pub staging_service_key: String,
    #[serde(default = "get_subscription_plan")]
    pub subscription_plan: String,
}

fn get_subscription_plan() -> String {
    "Basic".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilerConfig {
    #[serde(default = "get_enabled_extensions")]
    pub enabled_extensions: Vec<String>,
}

fn get_enabled_extensions() -> Vec<String> {
    vec!["tables".into(), "footnotes".into()]
}

impl Default for CompilerConfig {
    fn default() -> Self {
        Self {
            enabled_extensions: get_enabled_extensions(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetsConfig {
    #[serde(default = "get_audio_formats")]
    pub audio_formats: Vec<String>,
    #[serde(default = "get_image_formats")]
    pub image_formats: Vec<String>,
}

fn get_audio_formats() -> Vec<String> {
    vec!["mp3".into(), "wav".into(), "m4a".into()]
}

fn get_image_formats() -> Vec<String> {
    vec!["jpg".into(), "png".into()]
}

impl Default for AssetsConfig {
    fn default() -> Self {
        Self {
            audio_formats: get_audio_formats(),
            image_formats: get_image_formats(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    #[serde(default = "get_strict")]
    pub strict: bool,
}

fn get_strict() -> bool {
    true
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            strict: get_strict(),
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
        assert!(!m.project.artist_id.is_empty());
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
            m.backend.as_ref().unwrap().staging_url,
            "https://example.com"
        );
    }
}
