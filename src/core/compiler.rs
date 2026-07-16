use std::path::PathBuf;

use crate::core::CiteError;
use crate::core::cache::{self, BuildCache};
use crate::core::metadata::{Podcast, TimelineEntry};
use crate::core::project::ProjectContext;
use serde::Serialize;
use tracing::info;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct ContentBundle {
    pub compiler_version: f64,
    pub project: String,
    pub artist_id: String,
    pub podcasts: Vec<BundlePodcast>,
    pub timelines: Vec<BundleTimeline>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BundlePodcast {
    pub id: String,
    #[serde(flatten)]
    pub podcast: Podcast,
    pub content: Option<String>,
}

impl From<&Podcast> for BundlePodcast {
    fn from(p: &Podcast) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            podcast: p.clone(),
            content: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BundleTimeline {
    pub id: String,
    pub entries: Vec<TimelineEntry>,
}

pub enum CompileOutcome {
    UpToDate,
    Complete { podcasts: usize, artifact: PathBuf },
}

impl CompileOutcome {
    pub fn emit(&self) {
        match self {
            CompileOutcome::UpToDate => {
                info!("Nothing to rebuild — all files up to date");
            }
            CompileOutcome::Complete { podcasts, artifact } => {
                info!("Built {} podcast items", podcasts);
                info!("Build artifact at {}", artifact.display());
                info!("Build complete");
            }
        }
    }
}

pub async fn compile(ctx: &ProjectContext, force: bool) -> Result<CompileOutcome, CiteError> {
    let cache_path = ctx.cache_path();
    let content_files = ctx.content_files();

    let current_hashes = cache::hash_files(&content_files).await?;

    if !force {
        let cache = BuildCache::load_or_default(&cache_path).await?;
        if cache.compiler_version == ctx.manifest.build.compiler_version {
            let changed_hashes = cache.changed_since(&current_hashes);
            if changed_hashes.is_empty() {
                let result = CompileOutcome::UpToDate;
                result.emit();
                return Ok(result);
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

    let result = CompileOutcome::Complete {
        podcasts: ctx.metadata.podcasts.len(),
        artifact: build_dir.join("content.json"),
    };
    result.emit();
    Ok(result)
}

async fn build_bundle(ctx: &ProjectContext) -> Result<ContentBundle, CiteError> {
    let mut podcasts: Vec<BundlePodcast> = ctx
        .metadata
        .podcasts
        .iter()
        .map(BundlePodcast::from)
        .collect();
    let mut timelines = Vec::new();

    let normalize_asset = |path: &str| format!("assets/{}", path.trim_start_matches("assets/"));

    for item in &mut podcasts {
        if !item.podcast.file.is_empty() {
            let src = ctx.root.join(&item.podcast.file);
            if src.exists() && src.is_file() {
                let raw = tokio::fs::read_to_string(&src).await?;
                item.content = Some(raw);
            }
        }

        item.podcast.thumbnail = item.podcast.thumbnail.as_deref().map(normalize_asset);
        item.podcast.audio = item.podcast.audio.as_deref().map(normalize_asset);
    }

    for p in &ctx.metadata.podcasts {
        if let Some(citation) = &p.citation {
            let bib_src = ctx.root.join(citation);
            if bib_src.exists() {
                let bib_content = tokio::fs::read_to_string(&bib_src).await?;
                let entries = parse_bibtex(&bib_content);
                if !entries.is_empty() {
                    timelines.push(BundleTimeline {
                        id: Uuid::new_v4().to_string(),
                        entries,
                    });
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
        let entry_type = content[pos..open].trim().to_lowercase();
        if matches!(
            entry_type.as_str(),
            "comment" | "string" | "preamble" | "xdata"
        ) {
            let mut depth = 1;
            for (offset, &b) in bytes[open + 1..].iter().enumerate() {
                match b {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            pos = open + 1 + offset + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            continue;
        }
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
            id,
            date: Some(date),
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
            "nov" | "novermber" => "11",
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
    let cleaned: String = title.chars().filter(|&c| c != '{' && c != '}').collect();
    if author.is_empty() {
        cleaned
    } else {
        format!("{} — {}", cleaned, author)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(entries[0].date.as_deref(), Some("1935-05"));
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

    #[test]
    fn test_format_title_with_author() {
        assert_eq!(
            format_title("My Paper", "Smith, J."),
            "My Paper — Smith, J."
        );
    }

    #[test]
    fn test_format_title_without_author() {
        assert_eq!(format_title("My Paper", ""), "My Paper");
    }

    #[test]
    fn test_format_title_without_title() {
        assert_eq!(format_title("", "Smith, J."), "Smith, J.");
    }

    #[test]
    fn test_format_title_trims_surrounding_braces() {
        assert_eq!(format_title("{E}nsemble {M}ethods", ""), "Ensemble Methods");
    }

    #[test]
    fn test_format_bib_date_year_only() {
        let year = Some("2023".to_string());
        let month = None;
        assert_eq!(format_bib_date(&year, &month), "2023");
    }

    #[test]
    fn test_format_bib_date_year_month() {
        let year = Some("2023".to_string());
        let month = Some("may".to_string());
        assert_eq!(format_bib_date(&year, &month), "2023-05");
    }

    #[test]
    fn test_format_bib_date_full_month() {
        let year = Some("2023".to_string());
        let month = Some("January".to_string());
        assert_eq!(format_bib_date(&year, &month), "2023-01");
    }

    #[test]
    fn test_format_bib_date_empty() {
        let year = None;
        let month = None;
        assert_eq!(format_bib_date(&year, &month), "");
    }

    #[test]
    fn test_format_bib_date_invalid_month() {
        let year = Some("2023".to_string());
        let month = Some("invalid".to_string());
        assert_eq!(format_bib_date(&year, &month), "2023");
    }

    #[test]
    fn test_parse_bibtex_protective_braces() {
        let bib = r#"
@article{test,
  title = {The Great Paper},
  year = {2023},
}
"#;
        let entries = parse_bibtex(bib);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "The Great Paper");
    }

    #[test]
    fn test_parse_bibtex_entry_without_title() {
        let bib = r#"
@misc{no-title,
  author = {Doe, J.},
  year = {2023},
}
"#;
        let entries = parse_bibtex(bib);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].title.contains("Doe"));
    }

    #[test]
    fn test_parse_bibtex_no_url_or_doi() {
        let bib = r#"
@article{minimal,
  title = {Minimal Entry},
}
"#;
        let entries = parse_bibtex(bib);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url.as_deref(), Some(""));
    }

    #[test]
    fn test_parse_bibtex_skips_non_entries() {
        let bib = r#"
@comment{ this should be ignored }
@string{ key = "value" }
@preamble{ "x" }
@article{real,
  title = {Real Entry},
  year = {2023},
}
"#;
        let entries = parse_bibtex(bib);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].title.contains("Real Entry"));
    }
}
