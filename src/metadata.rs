use crate::slug::Slug;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Metadata {
    #[serde(default)]
    pub artists: Vec<Artist>,
    #[serde(default)]
    pub news: Vec<News>,
    #[serde(default)]
    pub podcasts: Vec<Podcast>,
    #[serde(default)]
    pub newsletters: Vec<Newsletter>,
    #[serde(default)]
    pub timelines: Vec<Timeline>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    pub slug: Slug,
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct News {
    pub slug: Slug,
    pub title: String,
    pub file: String,
    pub citation: Option<String>,
    pub category: Option<String>,
    #[serde(default)]
    pub artists: Vec<Slug>,
    #[serde(default)]
    pub podcasts: Vec<Slug>,
    #[serde(default)]
    pub timelines: Vec<Slug>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Podcast {
    pub slug: Slug,
    pub title: String,
    pub file: String,
    pub duration_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Newsletter {
    pub slug: Slug,
    pub title: String,
    pub issue_number: Option<u64>,
    pub published_date: Option<String>,
    #[serde(default)]
    pub included_news: Vec<Slug>,
    pub file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timeline {
    pub slug: Slug,
    pub title: String,
    #[serde(default)]
    pub entries: Vec<TimelineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub date: String,
    pub title: String,
    pub summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slug::Slug;

    fn slug(s: &str) -> Slug {
        Slug::new(s).unwrap()
    }

    #[test]
    fn test_all_slugs_covers_all_types() {
        let meta = Metadata {
            artists: vec![Artist { slug: slug("alice"), name: "Alice".into(), email: None }],
            news: vec![News {
                slug: slug("article-1"), title: "A".into(), file: "x.md".into(),
                citation: None, category: None, artists: vec![], podcasts: vec![],
                timelines: vec![], content: None,
            }],
            podcasts: vec![Podcast { slug: slug("pod-1"), title: "P".into(), file: "a.mp3".into(), duration_seconds: None }],
            newsletters: vec![Newsletter { slug: slug("nl-1"), title: "N".into(), issue_number: None, published_date: None, included_news: vec![], file: None }],
            timelines: vec![Timeline { slug: slug("tl-1"), title: "T".into(), entries: vec![] }],
        };
        let slugs = meta.all_slugs();
        assert_eq!(slugs.len(), 5);
        assert!(slugs.contains(&("artists", &slug("alice"))));
        assert!(slugs.contains(&("news", &slug("article-1"))));
        assert!(slugs.contains(&("podcasts", &slug("pod-1"))));
        assert!(slugs.contains(&("newsletters", &slug("nl-1"))));
        assert!(slugs.contains(&("timelines", &slug("tl-1"))));
    }

    #[test]
    fn test_yaml_roundtrip() {
        let yaml = r#"
artists:
  - slug: alice
    name: "Alice"
news: []
podcasts: []
newsletters: []
timelines: []
"#;
        let meta: Metadata = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(meta.artists.len(), 1);
        assert_eq!(meta.artists[0].slug.as_str(), "alice");
        assert_eq!(meta.artists[0].name, "Alice");
    }
}

/// A flat, deployable content bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBundle {
    pub compiler_version: String,
    pub project: String,
    pub artists: Vec<Artist>,
    pub news: Vec<News>,
    pub podcasts: Vec<Podcast>,
    pub newsletters: Vec<Newsletter>,
    pub timelines: Vec<Timeline>,
}
