use crate::cache::{self, BuildCache};
use crate::error::{CiteError, ValidationReport};
use crate::metadata::{ContentBundle, Timeline, TimelineEntry};
use crate::project::ProjectContext;
use crate::slug::Slug;
use std::collections::HashSet;
use tracing::instrument;

#[instrument(skip(ctx), fields(project = %ctx.manifest.project.name, force))]
pub async fn build(ctx: &ProjectContext, force: bool) -> Result<ValidationReport, CiteError> {
    let cache_path = ctx.cache_path();
    let content_files = ctx.content_files();

    let current_hashes = cache::hash_files(&content_files).await?;

    let changed = if force {
        ctx.metadata
            .news
            .iter()
            .map(|n| n.file.clone())
            .collect::<Vec<_>>()
    } else {
        let cache = BuildCache::load_or_default(&cache_path).await?;
        if cache.compiler_version != ctx.manifest.build.compiler_version {
            ctx.metadata.news.iter().map(|n| n.file.clone()).collect()
        } else {
            let changed_hashes = cache.changed_since(&current_hashes);
            if changed_hashes.is_empty() {
                return Ok(ValidationReport::new());
            }
            changed_hashes
        }
    };

    if changed.is_empty() && !force {
        return Ok(ValidationReport::new());
    }

    // Build the content bundle with transformations
    let bundle = build_bundle(ctx).await?;

    let build_dir = ctx.build_dir();
    tokio::fs::create_dir_all(&build_dir).await?;
    let json = serde_json::to_string_pretty(&bundle)?;
    tokio::fs::write(build_dir.join("content.json"), json).await?;

    copy_assets(ctx).await?;

    let cache = BuildCache::new(&ctx.manifest.build.compiler_version, current_hashes);
    cache.save(&cache_path).await?;

    let mut report = ValidationReport::new();
    report.info(format!("Built {} news items", ctx.metadata.news.len()));
    report.info(format!(
        "Build artifact at {}",
        build_dir.join("content.json").display()
    ));
    Ok(report)
}

async fn build_bundle(ctx: &ProjectContext) -> Result<ContentBundle, CiteError> {
    let all_slugs: HashSet<&str> = ctx
        .metadata
        .all_slugs()
        .iter()
        .map(|(_, s)| s.as_str())
        .collect();

    let mut news = ctx.metadata.news.clone();
    let mut timelines = Vec::new();

    for item in &mut news {
        let src = ctx.root.join(&item.file);
        if src.exists() {
            let raw = tokio::fs::read_to_string(&src).await.unwrap_or_default();
            let processed = resolve_wiki_links(&raw, &all_slugs);
            item.content = Some(processed);
        }
        // Rewrite file path to point to build/assets/
        item.file = format!("assets/{}", item.file);

        // Generate timeline from .bib citation if present
        if let Some(citation) = &item.citation {
            let bib_src = ctx.root.join(citation);
            if bib_src.exists() {
                let bib_content = tokio::fs::read_to_string(&bib_src)
                    .await
                    .unwrap_or_default();
                let entries = parse_bibtex(&bib_content);
                if !entries.is_empty() {
                    let slug_str = format!("{}-timeline", item.slug.as_str());
                    if let Ok(tl_slug) = Slug::new(&slug_str) {
                        timelines.push(Timeline {
                            slug: tl_slug,
                            title: format!("{} Timeline", item.title),
                            entries,
                        });
                    }
                }
            }
        }
    }

    // Rewrite podcast file paths
    let mut podcasts = ctx.metadata.podcasts.clone();
    for pod in &mut podcasts {
        pod.file = format!("assets/{}", pod.file);
    }

    Ok(ContentBundle {
        compiler_version: ctx.manifest.build.compiler_version.clone(),
        project: ctx.manifest.project.name.clone(),
        artists: ctx.metadata.artists.clone(),
        news,
        podcasts,
        newsletters: ctx.metadata.newsletters.clone(),
        timelines,
    })
}

fn parse_bibtex(content: &str) -> Vec<TimelineEntry> {
    let mut entries = Vec::new();
    let mut pos = 0;
    let bytes = content.as_bytes();

    while pos < bytes.len() {
        // Find @ symbol
        if bytes[pos] != b'@' {
            pos += 1;
            continue;
        }
        pos += 1;

        // Find opening brace
        let open = match content[pos..].find('{') {
            Some(i) => pos + i,
            None => break,
        };
        pos = open + 1;

        // Find matching closing brace with nesting
        let mut depth = 1;
        let mut close = None;
        for (offset, &b) in bytes[pos..].iter().enumerate() {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(pos + offset);
                        break;
                    }
                }
                _ => {}
            }
        }

        let end = match close {
            Some(i) => i,
            None => break,
        };

        let body = &content[pos..end];
        pos = end + 1;

        // Extract fields
        let title = extract_bib_field(body, "title").unwrap_or_default();
        let author = extract_bib_field(body, "author").unwrap_or_default();
        let year = extract_bib_field(body, "year");
        let month = extract_bib_field(body, "month");
        let summary = extract_bib_field(body, "abstract")
            .or_else(|| extract_bib_field(body, "note"))
            .unwrap_or_default();

        let date = format_bib_date(&year, &month);
        let entry_title = format_title(&title, &author);

        entries.push(TimelineEntry {
            date,
            title: entry_title,
            summary,
        });
    }

    entries
}

fn extract_bib_field(body: &str, field: &str) -> Option<String> {
    // Search for `field` preceded by a newline (with optional whitespace between)
    let bytes = body.as_bytes();
    let mut pos = 0;

    loop {
        // Find the field name
        let fpos = body[pos..].find(field)?;
        let abs_pos = pos + fpos;

        // Must be preceded by whitespace (newline, space, tab) or at start of body
        if abs_pos > 0 {
            let prev = bytes[abs_pos - 1];
            if prev != b'\n' && prev != b' ' && prev != b'\t' {
                pos = abs_pos + 1;
                continue;
            }
        }

        // Must be followed by optional whitespace and `=`
        let after_field = &body[abs_pos + field.len()..];
        let trimmed = after_field.trim_start();
        if !trimmed.starts_with('=') {
            pos = abs_pos + 1;
            continue;
        }

        let after_eq = trimmed[1..].trim();
        let val: &str = if let Some(inner) = after_eq.strip_prefix('{') {
            let mut depth = 1usize;
            let mut end = None;
            for (i, c) in inner.char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            end.map(|i| &inner[..i])?
        } else if let Some(quoted) = after_eq.strip_prefix('"') {
            let close = quoted.find('"')?;
            &quoted[..close]
        } else {
            // Unquoted value (until comma or newline or closing brace)
            let delim = after_eq.find([',', '}', '\n'])?;
            after_eq[..delim].trim()
        };

        let cleaned = val.trim().trim_end_matches(',');
        return Some(cleaned.to_string());
    }
}

fn format_bib_date(year: &Option<String>, month: &Option<String>) -> String {
    let y = year.as_deref().unwrap_or("");
    let m = month.as_deref().and_then(|m| {
        let m = m.trim().to_lowercase();
        Some(match m.as_str() {
            "jan" | "january" => "01",
            "feb" | "february" => "02",
            "mar" | "march" => "03",
            "apr" | "april" => "04",
            "may" => "05",
            "jun" | "june" => "06",
            "jul" | "july" => "07",
            "aug" | "august" => "08",
            "sep" | "september" => "09",
            "oct" | "october" => "10",
            "nov" | "november" => "11",
            "dec" | "december" => "12",
            _ => return None,
        })
    });

    match (y, m) {
        (y, Some(m)) if !y.is_empty() => format!("{y}-{m}"),
        (y, _) if !y.is_empty() => y.to_string(),
        _ => String::new(),
    }
}

fn format_title(title: &str, author: &str) -> String {
    if title.is_empty() {
        return author.to_string();
    }
    // Clean surrounding braces from BibTeX protection
    let cleaned = title
        .trim_matches(|c: char| c == '{' || c == '}')
        .to_string();
    if author.is_empty() {
        cleaned
    } else {
        format!("{} — {}", cleaned, author)
    }
}

async fn copy_assets(ctx: &ProjectContext) -> Result<(), CiteError> {
    let asset_dir = ctx.build_dir().join("assets");
    tokio::fs::create_dir_all(&asset_dir).await?;

    // Copy only files that deploy uploads to storage: news content + podcast audio
    let mut paths = Vec::new();
    for news_item in &ctx.metadata.news {
        paths.push(news_item.file.clone());
    }
    for pod in &ctx.metadata.podcasts {
        paths.push(pod.file.clone());
    }

    for file in &paths {
        let src = ctx.root.join(file);
        let dest = asset_dir.join(file);
        if src.exists() {
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::copy(&src, &dest).await?;
        }
    }

    Ok(())
}

fn resolve_wiki_links(content: &str, valid_slugs: &HashSet<&str>) -> String {
    let mut result = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("[[") {
        result.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        if let Some(end) = rest.find("]]") {
            let slug = rest[..end].trim();
            rest = &rest[end + 2..];
            if valid_slugs.contains(slug) {
                result.push_str(&format!("[{}]({{{{slug:{slug}}}}})", slug));
            } else {
                result.push_str(&format!("[[{slug}]]"));
            }
        } else {
            result.push_str("[[");
        }
    }
    result.push_str(rest);
    result
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

    #[test]
    fn test_parse_bibtex_extracts_timeline_entries() {
        let bib = r#"
@article{einstein1935,
  title = {Can Quantum-Mechanical Description of Physical Reality Be Considered Complete?},
  author = {Einstein, A. and Podolsky, B. and Rosen, N.},
  year = {1935},
  month = may,
  abstract = {A description of physical reality},
}
"#;
        let entries = parse_bibtex(bib);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].date, "1935-05");
        assert!(entries[0].title.contains("Quantum-Mechanical"));
    }

    #[test]
    fn test_parse_bibtex_empty() {
        assert!(parse_bibtex("").is_empty());
    }

    #[test]
    fn test_parse_bibtex_multiple_entries() {
        let bib = r#"
@article{first,
  title = {First Paper},
  year = {2020},
}
@article{second,
  title = {Second Paper},
  year = {2021},
}
"#;
        assert_eq!(parse_bibtex(bib).len(), 2);
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
                    content: None,
                }],
                ..Default::default()
            },
        };
        let report = build(&ctx, false).await.unwrap();
        assert!(!report.has_errors());
        assert!(ctx.build_dir().join("content.json").exists());

        // Verify content was embedded
        let json_str = std::fs::read_to_string(ctx.build_dir().join("content.json")).unwrap();
        let bundle: ContentBundle = serde_json::from_str(&json_str).unwrap();
        assert_eq!(bundle.news.len(), 1);
        assert_eq!(bundle.news[0].content.as_deref(), Some("# Hello"));
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

        let r2 = build(&ctx, false).await.unwrap();
        assert!(!r2.has_errors());
        assert!(r2.infos.is_empty());

        let r3 = build(&ctx, true).await.unwrap();
        assert!(!r3.has_errors());
        assert!(!r3.infos.is_empty());
    }

    #[tokio::test]
    async fn test_build_with_wiki_links() {
        let dir = tempfile::tempdir().unwrap();
        let content_dir = dir.path().join("content");
        std::fs::create_dir_all(&content_dir).unwrap();
        std::fs::write(
            content_dir.join("main.md"),
            "See [[ai-article]] for details",
        )
        .unwrap();

        let ctx = ProjectContext {
            root: dir.path().to_path_buf(),
            manifest: Manifest::default_template("test"),
            metadata: Metadata {
                news: vec![
                    News {
                        slug: make_slug("main"),
                        title: "Main".into(),
                        file: "content/main.md".into(),
                        citation: None,
                        category: None,
                        artists: vec![],
                        podcasts: vec![],
                        content: None,
                    },
                    News {
                        slug: make_slug("ai-article"),
                        title: "AI Article".into(),
                        file: "content/ai.md".into(),
                        citation: None,
                        category: None,
                        artists: vec![],
                        podcasts: vec![],
                        content: None,
                    },
                ],
                ..Default::default()
            },
        };

        let report = build(&ctx, false).await.unwrap();
        assert!(!report.has_errors());

        let json_str = std::fs::read_to_string(ctx.build_dir().join("content.json")).unwrap();
        let bundle: ContentBundle = serde_json::from_str(&json_str).unwrap();
        let main = bundle
            .news
            .iter()
            .find(|n| n.slug.as_str() == "main")
            .unwrap();
        assert!(
            main.content
                .as_deref()
                .unwrap()
                .contains("{{slug:ai-article}}")
        );
    }
}
