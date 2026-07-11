use colored::Colorize;

use crate::error::CiteError;

const REPO: &str = "VinayIN/cite-cli";
const BIN_NAME: &str = "cite-cli";

pub async fn upgrade() -> Result<(), CiteError> {
    let current_exe = std::env::current_exe()
        .map_err(|e| CiteError::Config(format!("Cannot determine executable path: {e}")))?;

    eprintln!("{}", "Checking for updates".bold());

    let client = reqwest::Client::new();
    let api_url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = client
        .get(&api_url)
        .header("User-Agent", "cite-cli")
        .header("Accept", "application/json")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(CiteError::Config(format!(
            "GitHub API returned HTTP {}",
            resp.status()
        )));
    }

    let release: serde_json::Value = resp.json().await?;
    let tag_name = release["tag_name"]
        .as_str()
        .ok_or_else(|| CiteError::Config("Could not parse latest release tag".into()))?;

    let latest = tag_name.strip_prefix('v').unwrap_or(tag_name);
    let current = env!("CARGO_PKG_VERSION");

    if current == latest {
        eprintln!(
            "{}",
            format!("Already up to date (v{current})").green().bold()
        );
        return Ok(());
    }

    if !is_newer(latest, current) {
        eprintln!(
            "{}",
            format!("Local version v{current} is newer than remote v{latest}").cyan()
        );
        return Ok(());
    }

    eprintln!(
        "{}",
        format!("New version available: v{latest} (current: v{current})")
            .yellow()
            .bold()
    );

    let target = target_triple();
    let download_url =
        format!("https://github.com/{REPO}/releases/download/v{latest}/{BIN_NAME}-{target}");

    eprintln!(
        "{}",
        format!("Downloading {BIN_NAME} v{latest} for {target}").bold()
    );

    let tmp_path = {
        let mut p = current_exe.clone();
        p.set_extension("tmp");
        p
    };

    let dl_resp = client.get(&download_url).send().await?;

    if !dl_resp.status().is_success() {
        return Err(CiteError::Config(format!(
            "Download failed: HTTP {}",
            dl_resp.status()
        )));
    }

    let bytes = dl_resp.bytes().await?;
    std::fs::write(&tmp_path, &bytes)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))?;
    }

    std::fs::rename(&tmp_path, &current_exe)?;

    eprintln!("{}", format!("Updated to v{latest}").green().bold());
    Ok(())
}

fn target_triple() -> String {
    let os = match std::env::consts::OS {
        "linux" => "unknown-linux-gnu",
        "macos" => "apple-darwin",
        "windows" => "pc-windows-msvc",
        other => other,
    };
    format!("{}-{os}", std::env::consts::ARCH)
}

fn is_newer(a: &str, b: &str) -> bool {
    parse_version(a) > parse_version(b)
}

fn parse_version(v: &str) -> (u64, u64, u64, u64) {
    let clean = v.split_once('-').map(|(base, _)| base).unwrap_or(v);
    let parts: Vec<&str> = clean.splitn(3, '.').collect();
    (
        parts.first().and_then(|s| s.parse().ok()).unwrap_or(0),
        parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0),
        parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
        if v.contains('-') { 1 } else { 0 },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version() {
        assert_eq!(parse_version("0.1.0"), (0, 1, 0, 0));
        assert_eq!(parse_version("1.0.0"), (1, 0, 0, 0));
        assert_eq!(parse_version("0.0.1"), (0, 0, 1, 0));
        assert_eq!(parse_version("2.5.3"), (2, 5, 3, 0));
        assert_eq!(parse_version("0.1.0-alpha"), (0, 1, 0, 1));
        assert_eq!(parse_version("0.1.0-rc.1"), (0, 1, 0, 1));
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.2.0", "0.1.99"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "1.0.0"));
        assert!(!is_newer("0.1.0", "0.1.0-alpha"));
        assert!(is_newer("0.1.0-alpha", "0.0.9"));
    }

    #[test]
    fn test_target_triple_format() {
        let triple = target_triple();
        assert!(
            triple.contains('-'),
            "target triple should contain a hyphen"
        );
    }
}
