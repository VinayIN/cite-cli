use colored::Colorize;
use std::path::Path;
use thiserror::Error;
use tracing::{info, warn};

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

pub enum Style {
    Success,
    Error,
    Warning,
    Info,
    Header,
}

pub fn styled(msg: impl AsRef<str>, style: Style) -> String {
    let s = msg.as_ref();
    match style {
        Style::Success => format!("  {}", s.green().bold()),
        Style::Error => format!("  {}", s.red().bold()),
        Style::Warning => format!("  {}", s.yellow().bold()),
        Style::Info => format!("  {}", s.cyan()),
        Style::Header => s.bold().underline().to_string(),
    }
}

pub fn check_file(root: &Path, filename: &str, hint: &str) {
    let path = root.join(filename);
    if path.exists() {
        info!("{filename} found");
    } else if hint.is_empty() {
        warn!("{filename} not found");
    } else {
        warn!("{filename} not found - {hint}");
    }
}

pub struct DoctorReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl DoctorReport {
    pub fn new() -> Self {
        Self {
            errors: vec![],
            warnings: vec![],
        }
    }

    pub fn error<S: Into<String>>(&mut self, msg: S) {
        self.errors.push(msg.into());
    }

    pub fn warning<S: Into<String>>(&mut self, msg: S) {
        self.warnings.push(msg.into());
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    pub fn print(&self) {
        for e in &self.errors {
            eprintln!("{}", styled(e, Style::Error));
        }
        for w in &self.warnings {
            eprintln!("{}", styled(w, Style::Warning));
        }

        let summary = format!(
            "{} error(s), {} warning(s)",
            self.errors.len(),
            self.warnings.len(),
        );
        if self.has_errors() {
            eprintln!("{}", styled(summary, Style::Error));
        } else if self.has_warnings() {
            eprintln!("{}", styled(summary, Style::Warning));
        } else {
            eprintln!("{}", styled(summary, Style::Success));
        }
    }
}

pub struct CompileReport {
    pub infos: Vec<String>,
}

impl CompileReport {
    pub fn new() -> Self {
        Self { infos: vec![] }
    }

    pub fn info<S: Into<String>>(&mut self, msg: S) {
        self.infos.push(msg.into());
    }

    pub fn print(&self) {
        for i in &self.infos {
            eprintln!("{}", styled(i, Style::Info));
        }

        eprintln!(
            "{}",
            styled(format!("{} info(s)", self.infos.len()), Style::Success)
        );
    }
}
