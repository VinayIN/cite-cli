use colored::Colorize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CiteError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Deploy error: {0}")]
    Deploy(String),

    #[error("Network error: {0}")]
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

pub struct ValidationReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub infos: Vec<String>,
}

impl ValidationReport {
    pub fn new() -> Self {
        Self {
            errors: vec![],
            warnings: vec![],
            infos: vec![],
        }
    }

    pub fn error<S: Into<String>>(&mut self, msg: S) {
        self.errors.push(msg.into());
    }

    pub fn warning<S: Into<String>>(&mut self, msg: S) {
        self.warnings.push(msg.into());
    }

    pub fn info<S: Into<String>>(&mut self, msg: S) {
        self.infos.push(msg.into());
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn print(&self) {
        for e in &self.errors {
            eprintln!("{}", format!("  ✖ {}", e).red().bold());
        }
        for w in &self.warnings {
            eprintln!("{}", format!("  ⚠ {}", w).yellow().bold());
        }
        for i in &self.infos {
            eprintln!("{}", format!("  ℹ {}", i).cyan());
        }

        let summary = format!(
            "{} error(s), {} warning(s), {} info(s)",
            self.errors.len(),
            self.warnings.len(),
            self.infos.len()
        );
        if self.has_errors() {
            eprintln!("{}", summary.red().bold());
        } else if self.has_warnings() {
            eprintln!("{}", summary.yellow().bold());
        } else {
            eprintln!("{}", summary.green().bold());
        }
    }

    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}
