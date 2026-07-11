use crate::cache::{self, BuildCache};
use crate::error::{CiteError, ValidationReport};
use crate::metadata::{ContentBundle, Timeline, TimelineEntry};
use crate::project::ProjectContext;
use tracing::instrument;
use uuid::Uuid;

#[instrument(skip(ctx), fields(project = %ctx.manifest.project.name, force))]
pub async fn compile(ctx: &ProjectContext, force: bool) -> Result<ValidationReport, CiteError> {
    let cache_path = ctx.cache_path();
    let content_files = ctx.content_files();

    let current_hashes = cache::hash_files(&content_files).await?;

    if !force {
        let cache = BuildCache::load_or_default(&cache_path).await?;
        if cache.compiler_version == ctx.manifest.build.compiler_version {
            let changed_hashes = cache.changed_since(&current_hashes);
            if changed_hashes.is_empty() {
                return Ok(ValidationReport::new());
            }
        }
    }

    let bundle = build_bundle(ctx).await?;

    let build_dir = ctx.build_dir();
    tokio::fs::create_dir_all(&build_dir).await?;
    let json = serde_json::to_string_pretty(&bundle)?;
    tokio::fs::write(build_dir.join("content.json"), json).await?;

    let cache = BuildCache::new(ctx.manifest.build.compiler_version, current_hashes);
    cache.save(&cache_path).await?;

    let mut report = ValidationReport::new();
    report.info(format!(
        "Built {} podcast items",
        ctx.metadata.podcasts.len()
    ));
    report.info(format!(
        "Build artifact at {}",
        build_dir.join("content.json").display()
    ));
    Ok(report)
}

async fn build_bundle(ctx: &ProjectContext) -> Result<ContentBundle, CiteError> {
    let mut podcasts = ctx.metadata.podcasts.clone();
    let mut timelines = Vec::new();

    for item in &mut podcasts {
        if !item.file.is_empty() {
            let src = ctx.root.join(&item.file);
            if src.exists() && src.is_file() {
                let raw = tokio::fs::read_to_string(&src).await.unwrap_or_default();
                item.content = Some(raw);
            }
        }

        if let Some(thumbnail) = &item.thumbnail {
            item.thumbnail = Some(format!(
                "assets/{}",
                thumbnail.trim_start_matches("assets/")
            ));
        }

        if let Some(audio) = &item.audio {
            let rewritten = format!("assets/{}", audio.trim_start_matches("assets/"));
            item.audio = Some(rewritten);
        }

        if let Some(citation) = &item.citation {
            let bib_src = ctx.root.join(citation);
            if bib_src.exists() {
                let bib_content = tokio::fs::read_to_string(&bib_src)
                    .await
                    .unwrap_or_default();
                let entries = parse_bibtex(&bib_content);
                if !entries.is_empty() {
                    let id = Uuid::new_v4().to_string();
                    timelines.push(Timeline { id, entries });
                }
            }
        }
    }

    let artist_id = ctx.manifest.project.artist_id.clone();

    Ok(ContentBundle {
        compiler_version: ctx.manifest.build.compiler_version,
        project: ctx.manifest.project.name.clone(),
        artist_id,
        podcasts,
        timelines,
    })
}

fn parse_bibtex(content: &str) -> Vec<TimelineEntry> {
    let mut entries = Vec::new();
    let mut pos = 0;
    let bytes = content.as_bytes();

    while pos < bytes.len() {
        if bytes[pos] != b'@' {
            pos += 1;
            continue;
        }
        pos += 1;

        let open = match content[pos..].find('{') {
            Some(i) => pos + i,
            None => break,
        };
        pos = open + 1;

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

        let title = extract_bib_field(body, "title").unwrap_or_default();
        let author = extract_bib_field(body, "author").unwrap_or_default();
        let year = extract_bib_field(body, "year");
        let month = extract_bib_field(body, "month");
        let summary = extract_bib_field(body, "abstract")
            .or_else(|| extract_bib_field(body, "note"))
            .unwrap_or_default();
        let url = extract_bib_field(body, "url")
            .or_else(|| extract_bib_field(body, "doi"))
            .unwrap_or_default();
        let date = format_bib_date(&year, &month);
        let entry_title = format_title(&title, &author);
        let id = Uuid::new_v4().to_string();

        entries.push(TimelineEntry {
            date,
            id,
            title: entry_title,
            summary: Some(summary),
            url: Some(url),
        });
    }

    entries
}

fn extract_bib_field(body: &str, field: &str) -> Option<String> {
    let bytes = body.as_bytes();
    let mut pos = 0;

    loop {
        let fpos = body[pos..].find(field)?;
        let abs_pos = pos + fpos;

        if abs_pos > 0 {
            let prev = bytes[abs_pos - 1];
            if prev != b'\n' && prev != b' ' && prev != b'\t' {
                pos = abs_pos + 1;
                continue;
            }
        }

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
    let cleaned = title
        .trim_matches(|c: char| c == '{' || c == '}')
        .to_string();
    if author.is_empty() {
        cleaned
    } else {
        format!("{} — {}", cleaned, author)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::metadata::{Metadata, Podcast};
    use crate::project::ProjectContext;

    #[test]
    fn test_parse_bibtex_extracts_timeline_entries() {
        let bib = r#"
@article{einstein1935,
  title = {Can Quantum-Mechanical Description of Physical Reality Be Considered Complete?},
  author = {Einstein, A. and Podolsky, B. and Rosen, N.},
  year = {1935},
  month = may,
  abstract = {A description of physical reality},
  doi = {10.1038/35057060},
}
"#;
        let entries = parse_bibtex(bib);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].date, "1935-05");
        assert!(entries[0].title.contains("Quantum-Mechanical"));
        assert_eq!(entries[0].url.as_deref(), Some("10.1038/35057060"));
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
        let report = compile(&ctx, false).await.unwrap();
        assert!(!report.has_errors());
    }

    #[tokio::test]
    async fn test_build_creates_content_json() {
        let dir = tempfile::tempdir().unwrap();
        let content_dir = dir.path().join("content");
        std::fs::create_dir_all(&content_dir).unwrap();
        std::fs::write(content_dir.join("article.md"), "# Hello").unwrap();

        let mut manifest = Manifest::default_template("test");
        manifest.project.artist_id = "abc".into();
        let ctx = ProjectContext {
            root: dir.path().to_path_buf(),
            manifest,
            metadata: Metadata {
                podcasts: vec![Podcast {
                    id: "abc".into(),
                    title: "My Podcast".into(),
                    file: "content/article.md".into(),
                    source_url: None,
                    category: Some("tech".into()),
                    thumbnail: None,
                    audio: None,
                    citation: None,
                    content: None,
                }],
            },
        };
        let report = compile(&ctx, false).await.unwrap();
        assert!(!report.has_errors());
        assert!(ctx.build_dir().join("content.json").exists());

        let json_str = std::fs::read_to_string(ctx.build_dir().join("content.json")).unwrap();
        let bundle: ContentBundle = serde_json::from_str(&json_str).unwrap();
        assert_eq!(bundle.podcasts.len(), 1);
        assert_eq!(bundle.podcasts[0].id, "abc");
        assert_eq!(bundle.podcasts[0].content.as_deref(), Some("# Hello"));
        assert!(bundle.podcasts[0].citation.is_none());
        assert_eq!(bundle.podcasts[0].category.as_deref(), Some("tech"));
    }

    #[tokio::test]
    async fn test_build_force_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ProjectContext {
            root: dir.path().to_path_buf(),
            manifest: Manifest::default_template("test"),
            metadata: Metadata::default(),
        };
        let r1 = compile(&ctx, false).await.unwrap();
        assert!(!r1.has_errors());

        let r2 = compile(&ctx, false).await.unwrap();
        assert!(!r2.has_errors());
        assert!(r2.infos.is_empty());

        let r3 = compile(&ctx, true).await.unwrap();
        assert!(!r3.has_errors());
        assert!(!r3.infos.is_empty());
    }
}
