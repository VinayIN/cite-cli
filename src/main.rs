mod cli;
mod core;
mod tui;

use clap::Parser;
use cli::Cli;
use colored::Colorize;
use std::io::Write;
use tokio::sync::mpsc;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        EnvFilter::new("cite_cli=trace")
    } else {
        EnvFilter::new("cite_cli=info")
    };

    if let Some(cmd) = cli.command {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_level(true)
            .with_writer(std::io::stderr)
            .init();

        info!("cite-cli v{}", env!("CARGO_PKG_VERSION"));

        if let Err(e) = cmd.execute().await {
            eprintln!("{} {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    } else {
        let (log_tx, log_rx) = mpsc::unbounded_channel::<String>();
        let make_writer = move || LogWriter {
            tx: log_tx.clone(),
            buf: String::new(),
        };

        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_level(true)
            .with_ansi(false)
            .with_writer(make_writer)
            .init();

        info!("cite-cli v{}", env!("CARGO_PKG_VERSION"));

        if let Err(e) = tui::run_tui(log_rx).await {
            eprintln!("{} {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

struct LogWriter {
    tx: mpsc::UnboundedSender<String>,
    buf: String,
}

impl Write for LogWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        match std::str::from_utf8(data) {
            Ok(s) => self.buf.push_str(s),
            Err(_) => self.buf.push_str(&String::from_utf8_lossy(data)),
        }
        while let Some(pos) = self.buf.find('\n') {
            let line = self.buf[..pos].trim_end().to_string();
            self.buf.drain(..=pos);
            let _ = self.tx.send(line);
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buf.trim_end().is_empty() {
            let line = std::mem::take(&mut self.buf).trim_end().to_string();
            let _ = self.tx.send(line);
        }
        Ok(())
    }
}

impl Drop for LogWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}
