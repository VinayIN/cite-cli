use thiserror::Error;
#[derive(Error, Debug)]
pub enum CiteError {
    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Config(String),

    #[error("{0}")]
    Parse(String),

    #[error("{0}")]
    Auth(String),

    #[error("{0}")]
    Deploy(String),

    #[error("{0}")]
    Network(#[from] reqwest::Error),
}

impl From<serde_yaml::Error> for CiteError {
    fn from(e: serde_yaml::Error) -> Self {
        CiteError::Parse(format!("YAML parse error: {}", e))
    }
}

impl From<toml::de::Error> for CiteError {
    fn from(e: toml::de::Error) -> Self {
        CiteError::Parse(format!("TOML parse error: {}", e))
    }
}

impl From<serde_json::Error> for CiteError {
    fn from(e: serde_json::Error) -> Self {
        CiteError::Parse(format!("JSON error: {}", e))
    }
}
