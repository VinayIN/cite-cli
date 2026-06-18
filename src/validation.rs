use crate::error::ValidationReport;
use crate::project::ProjectContext;
use std::collections::HashSet;
use std::path::Path;
use tracing::instrument;

#[instrument(skip(ctx), fields(project = %ctx.manifest.project.name))]
pub fn validate_all(ctx: &ProjectContext) -> ValidationReport {
    let mut report = ValidationReport::new();

    validate_project_structure(ctx, &mut report);
    validate_slug_uniqueness(ctx, &mut report);
    validate_cross_references(ctx, &mut report);
    validate_file_existence(ctx, &mut report);
    validate_asset_formats(ctx, &mut report);
    validate_broken_links(ctx, &mut report);

    if ctx.manifest.validation.strict {
        enforce_strict_rules(ctx, &mut report);
    }

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
                "Required file '{}' not found at {}",
                name,
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

fn validate_slug_uniqueness(ctx: &ProjectContext, report: &mut ValidationReport) {
    let mut seen: HashSet<(&str, String)> = HashSet::new();
    for (content_type, slug) in ctx.metadata.all_slugs() {
        if !seen.insert((content_type, slug.to_string())) {
            report.error(format!("Duplicate slug '{}' in {}", slug, content_type));
        }
    }
}

fn validate_cross_references(ctx: &ProjectContext, report: &mut ValidationReport) {
    let valid_artists: HashSet<String> = ctx
        .metadata
        .artists
        .iter()
        .map(|a| a.slug.to_string())
        .collect();
    let valid_podcasts: HashSet<String> = ctx
        .metadata
        .podcasts
        .iter()
        .map(|p| p.slug.to_string())
        .collect();

    for news in &ctx.metadata.news {
        for ref_slug in &news.artists {
            if !valid_artists.contains(ref_slug.as_str()) {
                report.error(format!(
                    "News '{}' references unknown artist '{}'",
                    news.slug, ref_slug
                ));
            }
        }
        for ref_slug in &news.podcasts {
            if !valid_podcasts.contains(ref_slug.as_str()) {
                report.error(format!(
                    "News '{}' references unknown podcast '{}'",
                    news.slug, ref_slug
                ));
            }
        }
    }

}

fn validate_file_existence(ctx: &ProjectContext, report: &mut ValidationReport) {
    for news in &ctx.metadata.news {
        let path = ctx.root.join(&news.file);
        if !path.exists() {
            report.error(format!(
                "News '{}' references file '{}' which does not exist",
                news.slug, news.file
            ));
        }
        if let Some(cit) = &news.citation {
            let cit_path = ctx.root.join(cit);
            if !cit_path.exists() {
                report.warning(format!(
                    "News '{}' references citation file '{}' which does not exist",
                    news.slug, cit
                ));
            }
        }
    }

    for pod in &ctx.metadata.podcasts {
        let path = ctx.root.join(&pod.file);
        if !path.exists() {
            report.error(format!(
                "Podcast '{}' references audio file '{}' which does not exist",
                pod.slug, pod.file
            ));
        }
    }
}

fn validate_asset_formats(ctx: &ProjectContext, report: &mut ValidationReport) {
    let allowed_audio: HashSet<&str> = ctx
        .manifest
        .assets
        .audio_formats
        .iter()
        .map(|s| s.as_str())
        .collect();

    for pod in &ctx.metadata.podcasts {
        let ext = Path::new(&pod.file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !allowed_audio.contains(ext) {
            report.warning(format!(
                "Podcast '{}' has file '{}' with extension '{ext}' not in allowed audio formats {:?}",
                pod.slug, pod.file, ctx.manifest.assets.audio_formats
            ));
        }
    }

    // Check news content files for proper markdown extensions
    for news in &ctx.metadata.news {
        let ext = Path::new(&news.file)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !matches!(ext, "md" | "rst" | "bib") {
            report.warning(format!(
                "News '{}' has file '{}' with unexpected extension '{ext}'",
                news.slug, news.file
            ));
        }
    }
}

fn validate_broken_links(ctx: &ProjectContext, report: &mut ValidationReport) {
    let all_slugs: HashSet<&str> = ctx
        .metadata
        .all_slugs()
        .iter()
        .map(|(_, s)| s.as_str())
        .collect();

    for news in &ctx.metadata.news {
        let path = ctx.root.join(&news.file);
        if !path.exists() {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            // Detect wiki-style [[slug]] references
            for cap in content.split("[[") {
                if let Some(slug) = cap.split("]]").next() {
                    let slug = slug.trim();
                    if !slug.is_empty() && !all_slugs.contains(slug) {
                        report.warning(format!(
                            "News '{}' has broken wiki-link '[[{}]]' — no matching slug found",
                            news.slug, slug
                        ));
                    }
                }
            }
            // Detect markdown links to local files
            for cap in content.split("](") {
                if let Some(target) = cap.split(')').next() {
                    let target = target.trim();
                    if !target.starts_with("http") && !target.starts_with('#') && !target.is_empty()
                    {
                        let resolved = ctx.root.join(target);
                        if !resolved.exists() {
                            report.warning(format!(
                                "News '{}' has broken link to '{}' — file not found",
                                news.slug, target
                            ));
                        }
                    }
                }
            }
        }
    }
}

fn enforce_strict_rules(ctx: &ProjectContext, report: &mut ValidationReport) {
    for pod in &ctx.metadata.podcasts {
        if pod.duration_seconds.is_none() {
            report.error(format!(
                "Podcast '{}' has no duration_seconds set (strict mode)",
                pod.slug
            ));
        }
    }
}

#[instrument(skip(ctx), fields(project = %ctx.manifest.project.name))]
pub fn lint_all(ctx: &ProjectContext) -> ValidationReport {
    let mut report = ValidationReport::new();

    // Naming conventions
    for (content_type, slug) in ctx.metadata.all_slugs() {
        let s = slug.as_str();
        if s.chars().any(|c| c.is_uppercase()) {
            report.warning(format!(
                "{} slug '{}' should be kebab-case (lowercase)",
                content_type, s
            ));
        }
    }

    // Audio metadata
    for pod in &ctx.metadata.podcasts {
        if pod.duration_seconds.is_none() {
            report.warning(format!(
                "Podcast '{}' has no duration_seconds set",
                pod.slug
            ));
        }
    }

    // Word counts / basic content checks
    for news in &ctx.metadata.news {
        let path = ctx.root.join(&news.file);
        if path.exists()
            && let Ok(content) = std::fs::read_to_string(&path)
        {
            let word_count = content.split_whitespace().count();
            if word_count < 10 {
                report.warning(format!(
                    "News '{}' is very short ({} words)",
                    news.slug, word_count
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
    use crate::slug::Slug;

    fn test_context(
        news_items: Vec<News>,
        artists: Vec<Artist>,
    ) -> (ProjectContext, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let manifest = Manifest::default_template("test");
        // Create required files and dirs so project structure validation passes
        std::fs::write(
            root.join("cite.toml"),
            toml::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("metadata.yml"),
            "artists: []\nnews: []\npodcasts: []\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("content")).unwrap();
        std::fs::create_dir_all(root.join("assets/audio")).unwrap();
        std::fs::create_dir_all(root.join("assets/images")).unwrap();
        let ctx = ProjectContext {
            root,
            manifest,
            metadata: Metadata {
                artists,
                news: news_items,
                ..Default::default()
            },
        };
        (ctx, dir)
    }

    fn make_slug(s: &str) -> Slug {
        Slug::new(s).unwrap()
    }

    #[test]
    fn test_validate_empty_metadata() {
        let (ctx, _dir) = test_context(vec![], vec![]);
        let report = validate_all(&ctx);
        assert!(!report.has_errors());
    }

    #[test]
    fn test_validate_duplicate_slug() {
        let news = vec![
            News {
                slug: make_slug("dup"),
                title: "First".into(),
                file: "content/a.md".into(),
                citation: None,
                category: None,
                artists: vec![],
                podcasts: vec![],
                content: None,
            },
            News {
                slug: make_slug("dup"),
                title: "Second".into(),
                file: "content/b.md".into(),
                citation: None,
                category: None,
                artists: vec![],
                podcasts: vec![],
                content: None,
            },
        ];
        let (ctx, _dir) = test_context(news, vec![]);
        let report = validate_all(&ctx);
        assert!(report.has_errors());
        assert!(report.errors.iter().any(|e| e.contains("Duplicate slug")));
    }

    #[test]
    fn test_validate_bad_cross_ref() {
        let news = vec![News {
            slug: make_slug("article-1"),
            title: "Article".into(),
            file: "content/a.md".into(),
            citation: None,
            category: None,
            artists: vec![make_slug("nonexistent")],
            podcasts: vec![],
            content: None,
        }];
        let (ctx, _dir) = test_context(news, vec![]);
        let report = validate_all(&ctx);
        assert!(report.has_errors());
        assert!(report.errors.iter().any(|e| e.contains("unknown artist")));
    }

    #[test]
    fn test_lint_uppercase_slug() {
        let news = vec![News {
            slug: Slug("UPPERCASE".to_string()),
            title: "Bad".into(),
            file: "content/b.md".into(),
            citation: None,
            category: None,
            artists: vec![],
            podcasts: vec![],
            content: None,
        }];
        let (ctx, _dir) = test_context(news, vec![]);
        let report = lint_all(&ctx);
        assert!(report.has_warnings());
        assert!(report.warnings.iter().any(|w| w.contains("kebab-case")));
    }

    #[test]
    fn test_lint_missing_duration() {
        use crate::metadata::Podcast;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let ctx = ProjectContext {
            root,
            manifest: Manifest::default_template("test"),
            metadata: Metadata {
                podcasts: vec![Podcast {
                    slug: make_slug("no-duration"),
                    title: "No Duration".into(),
                    file: "assets/audio/pod.mp3".into(),
                    duration_seconds: None,
                }],
                ..Default::default()
            },
        };
        let report = lint_all(&ctx);
        assert!(report.has_warnings());
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("no duration_seconds"))
        );
    }
}
