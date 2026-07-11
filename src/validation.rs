use crate::error::ValidationReport;
use crate::project::ProjectContext;
use std::path::Path;
use tracing::instrument;

#[instrument(skip(ctx), fields(project = %ctx.manifest.project.name))]
pub fn validate_all(ctx: &ProjectContext) -> ValidationReport {
    let mut report = ValidationReport::new();

    validate_project_structure(ctx, &mut report);
    validate_file_existence(ctx, &mut report);
    validate_asset_formats(ctx, &mut report);

    report
}

fn validate_project_structure(ctx: &ProjectContext, report: &mut ValidationReport) {
    let required = [
        ("cite.toml", ctx.root.join("cite.toml")),
        (
            &ctx.manifest.project.metadata_file,
            ctx.root.join(&ctx.manifest.project.metadata_file),
        ),
    ];
    for (name, path) in &required {
        if !path.exists() {
            report.error(format!(
                "Required file '{name}' not found at {}",
                path.display()
            ));
        }
    }

    let dirs = [
        ("content", ctx.content_dir()),
        ("assets/images", ctx.root.join("assets/images")),
        ("assets/audio", ctx.root.join("assets/audio")),
    ];
    for (name, path) in &dirs {
        if !path.is_dir() {
            report.warning(format!(
                "Directory '{name}' does not exist at {}",
                path.display()
            ));
        }
    }
}

fn validate_file_existence(ctx: &ProjectContext, report: &mut ValidationReport) {
    for pod in &ctx.metadata.podcasts {
        let path = ctx.root.join(&pod.file);
        if !path.exists() {
            report.error(format!(
                "Podcast '{}' references file '{}' which does not exist",
                pod.title, pod.file
            ));
        }

        if let Some(cit) = &pod.citation {
            let cit_path = ctx.root.join(cit);
            if !cit_path.exists() {
                report.warning(format!(
                    "Podcast '{}' references citation file '{}' which does not exist",
                    pod.title, cit
                ));
            }
        }

        if let Some(audio) = &pod.audio {
            let audio_path = ctx.root.join(audio);
            if !audio_path.exists() {
                report.error(format!(
                    "Podcast '{}' references audio file '{}' which does not exist",
                    pod.title, audio
                ));
            }
        }
    }
}

fn validate_asset_formats(ctx: &ProjectContext, report: &mut ValidationReport) {
    let allowed_audio: std::collections::HashSet<&str> = ctx
        .manifest
        .assets
        .audio_formats
        .iter()
        .map(|s| s.as_str())
        .collect();

    for pod in &ctx.metadata.podcasts {
        if let Some(audio) = &pod.audio {
            let ext = Path::new(audio)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if !allowed_audio.contains(ext) {
                report.warning(format!(
                    "Podcast '{}' has audio file '{audio}' with extension '{ext}' not in allowed audio formats {:?}",
                    pod.title, ctx.manifest.assets.audio_formats
                ));
            }
        }

        let ext = Path::new(&pod.file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !matches!(ext, "md" | "rst") {
            report.warning(format!(
                "Podcast '{}' has file '{}' with unexpected extension '{ext}'",
                pod.title, pod.file
            ));
        }
    }
}

#[instrument(skip(ctx), fields(project = %ctx.manifest.project.name))]
pub fn lint_all(ctx: &ProjectContext) -> ValidationReport {
    let mut report = ValidationReport::new();

    for pod in &ctx.metadata.podcasts {
        let path = ctx.root.join(&pod.file);
        if path.exists()
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            let word_count = content.split_whitespace().count();
            if word_count < 10 {
                report.warning(format!(
                    "Podcast '{}' is very short ({} words)",
                    pod.title, word_count
                ));
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::*;
    use crate::metadata::*;
    use crate::project::ProjectContext;

    fn test_context(podcasts: Vec<Podcast>) -> (ProjectContext, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let manifest = Manifest::default_template("test");
        std::fs::write(
            root.join("cite.toml"),
            toml::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("metadata.yml"), "podcasts: []\n").unwrap();
        std::fs::create_dir_all(root.join("content")).unwrap();
        std::fs::create_dir_all(root.join("assets/audio")).unwrap();
        std::fs::create_dir_all(root.join("assets/images")).unwrap();
        let ctx = ProjectContext {
            root,
            manifest,
            metadata: Metadata { podcasts },
        };
        (ctx, dir)
    }

    #[test]
    fn test_validate_empty_metadata() {
        let (ctx, _dir) = test_context(vec![]);
        let report = validate_all(&ctx);
        assert!(!report.has_errors());
    }

    #[test]
    fn test_validate_file_not_found() {
        let (ctx, _dir) = test_context(vec![Podcast {
            id: Some("abc".into()),
            title: "Missing".into(),
            file: "content/nonexistent.md".into(),
            source_url: None,
            category: None,
            thumbnail: None,
            audio: None,
            citation: None,
            content: None,
        }]);
        let report = validate_all(&ctx);
        assert!(report.has_errors());
        assert!(report.errors.iter().any(|e| e.contains("does not exist")));
    }

    #[test]
    fn test_lint_short_content() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("content")).unwrap();
        std::fs::write(root.join("content/short.md"), "Hi").unwrap();
        let ctx = ProjectContext {
            root,
            manifest: Manifest::default_template("test"),
            metadata: Metadata {
                podcasts: vec![Podcast {
                    id: Some("abc".into()),
                    title: "Short".into(),
                    file: "content/short.md".into(),
                    source_url: None,
                    category: None,
                    thumbnail: None,
                    audio: None,
                    citation: None,
                    content: None,
                }],
            },
        };
        let report = lint_all(&ctx);
        assert!(report.has_warnings());
        assert!(report.warnings.iter().any(|w| w.contains("very short")));
    }
}
