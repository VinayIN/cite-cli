use std::io::Write;

use colored::Colorize;

use crate::error::CiteError;

pub fn uninstall(force: bool) -> Result<(), CiteError> {
    let current_exe = std::env::current_exe()
        .map_err(|e| CiteError::Config(format!("Cannot determine executable path: {e}")))?;

    let install_dir = current_exe
        .parent()
        .ok_or_else(|| CiteError::Config("Cannot determine install directory".into()))?;

    eprintln!(
        "{}",
        format!("cite-cli is installed at: {}", current_exe.display()).bold()
    );

    if !force {
        eprintln!();
        eprintln!("This will delete the binary. Shell config files might NOT be modified.");
        eprintln!();
        eprint!("Are you sure? [y/N] ");
        std::io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        match input.trim().to_lowercase().as_str() {
            "y" | "yes" => {}
            _ => {
                eprintln!("{}", "Uninstall cancelled".red().bold());
                return Ok(());
            }
        }
    }

    std::fs::remove_file(&current_exe)?;
    eprintln!(
        "{}",
        format!("Removed {}", current_exe.display()).green().bold()
    );

    if install_dir
        .read_dir()
        .map(|mut d| d.next().is_none())
        .unwrap_or(false)
    {
        let _ = std::fs::remove_dir(install_dir);
        eprintln!(
            "{}",
            format!("Removed empty directory {}", install_dir.display())
                .green()
                .bold()
        );
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| "~".into());
    let shell_files = [
        format!("{home}/.zshrc"),
        format!("{home}/.bashrc"),
        format!("{home}/.bash_profile"),
        format!("{home}/.profile"),
    ];

    let install_dir_str = install_dir.to_string_lossy();
    let found = shell_files.iter().any(|p| {
        std::fs::read_to_string(p)
            .map(|c| c.contains(&*install_dir_str))
            .unwrap_or(false)
    });

    if found {
        eprintln!();
        eprintln!(
            "{}",
            "Shell config files contain PATH references to the install directory.".cyan()
        );
        eprintln!(
            "  Edit your shell config (~/.zshrc, ~/.bashrc, etc.) and remove lines containing:"
        );
        eprintln!("    {}", install_dir_str);
        eprintln!("  Then restart your shell or run: source ~/.zshrc");
    }

    eprintln!();
    eprintln!("{}", "cite-cli has been uninstalled.".green().bold());
    Ok(())
}
