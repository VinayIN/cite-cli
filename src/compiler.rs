use crate::cache::{self, BuildCache};
use crate::error::{CiteError, ValidationReport};
use crate::metadata::ContentBundle;
use crate::project::ProjectContext;
use tracing::instrument;

#[instrument(skip(ctx), fields(project = %ctx.manifest.project.name, force))]
pub async fn build(ctx: &ProjectContext, force: bool) -> Result<ValidationReport, CiteError> {
    let cache_path = ctx.cache_path();
    let content_files = ctx.content_files();

    // 1. Compute current file hashes
    let current_hashes = cache::hash_files(&content_files).await?;

    // 2. Determine changed files
    let changed = if force {
        ctx.metadata
            .news
            .iter()
            .map(|n| n.file.clone())
            .collect::<Vec<_>>()
    } else {
        let cache = BuildCache::load_or_default(&cache_path)?;
        if cache.compiler_version != ctx.manifest.build.compiler_version {
            // Full rebuild on version mismatch
            ctx.metadata.news.iter().map(|n| n.file.clone()).collect()
        } else {
            let changed_hashes = cache.changed_since(&current_hashes);
            if changed_hashes.is_empty() {
                return Ok(ValidationReport::new()); // No changes
            }
            changed_hashes
        }
    };

    if changed.is_empty() && !force {
        return Ok(ValidationReport::new());
    }

    // 3. Build the content bundle
    let bundle = ContentBundle {
        compiler_version: ctx.manifest.build.compiler_version.clone(),
        project: ctx.manifest.project.name.clone(),
        artists: ctx.metadata.artists.clone(),
        news: ctx.metadata.news.clone(),
        podcasts: ctx.metadata.podcasts.clone(),
        newsletters: ctx.metadata.newsletters.clone(),
        timelines: ctx.metadata.timelines.clone(),
    };

    // 4. Write build artifact
    let build_dir = ctx.build_dir();
    tokio::fs::create_dir_all(&build_dir).await?;
    let json = serde_json::to_string_pretty(&bundle)?;
    tokio::fs::write(build_dir.join("content.json"), json).await?;

    // 5. Copy referenced content files to build/assets
    let assets_dir = build_dir.join("assets");
    tokio::fs::create_dir_all(&assets_dir).await?;
    for news_item in &ctx.metadata.news {
        let src = ctx.root.join(&news_item.file);
        if src.exists() {
            let dest = assets_dir.join(&news_item.file);
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::copy(&src, &dest).await?;
        }
    }

    // 6. Persist new cache
    let cache = BuildCache::new(&ctx.manifest.build.compiler_version, current_hashes);
    cache.save(&cache_path)?;

    let mut report = ValidationReport::new();
    report.info(format!("Built {} news items", ctx.metadata.news.len()));
    report.info(format!(
        "Build artifact at {}",
        build_dir.join("content.json").display()
    ));
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::metadata::{Metadata, News};
    use crate::project::ProjectContext;
    use crate::slug::Slug;

    fn make_slug(s: &str) -> Slug {
        Slug::new(s).unwrap()
    }

    #[tokio::test]
    async fn test_build_empty_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ProjectContext {
            root: dir.path().to_path_buf(),
            manifest: Manifest::default_template("test"),
            metadata: Metadata::default(),
        };
        let report = build(&ctx, false).await.unwrap();
        assert!(!report.has_errors());
    }

    #[tokio::test]
    async fn test_build_creates_content_json() {
        let dir = tempfile::tempdir().unwrap();
        let content_dir = dir.path().join("content");
        std::fs::create_dir_all(&content_dir).unwrap();
        std::fs::write(content_dir.join("article.md"), "# Hello").unwrap();

        let ctx = ProjectContext {
            root: dir.path().to_path_buf(),
            manifest: Manifest::default_template("test"),
            metadata: Metadata {
                news: vec![News {
                    slug: make_slug("my-article"),
                    title: "My Article".into(),
                    file: "content/article.md".into(),
                    citation: None,
                    category: Some("tech".into()),
                    artists: vec![],
                    podcasts: vec![],
                    timelines: vec![],
                }],
                ..Default::default()
            },
        };
        let report = build(&ctx, false).await.unwrap();
        assert!(!report.has_errors());
        assert!(ctx.build_dir().join("content.json").exists());
        assert!(ctx.build_dir().join("assets/content/article.md").exists());
    }

    #[tokio::test]
    async fn test_build_force_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ProjectContext {
            root: dir.path().to_path_buf(),
            manifest: Manifest::default_template("test"),
            metadata: Metadata::default(),
        };
        let r1 = build(&ctx, false).await.unwrap();
        assert!(!r1.has_errors());

        // Cache hit — should succeed but be empty (no changes)
        let r2 = build(&ctx, false).await.unwrap();
        assert!(!r2.has_errors());
        assert!(r2.infos.is_empty());

        // Force rebuild
        let r3 = build(&ctx, true).await.unwrap();
        assert!(!r3.has_errors());
        assert!(!r3.infos.is_empty());
    }
}
