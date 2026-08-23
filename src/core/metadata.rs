use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TimelineItem {
    Citation(String),
    News(i64),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Podcast {
    pub title: String,
    pub file: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timeline: Vec<TimelineItem>,
}

impl Podcast {
    pub fn citation(&self) -> Option<&str> {
        self.timeline.iter().find_map(|item| match item {
            TimelineItem::Citation(path) => Some(path.as_str()),
            TimelineItem::News(_) => None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TimelineEntry {
    pub id: String,
    pub date: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub url: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

impl Default for TimelineEntry {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            date: None,
            title: String::new(),
            summary: None,
            url: None,
            link: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Metadata {
    pub podcasts: Vec<Podcast>,
}

impl Metadata {
    pub fn referenced_files(&self) -> Vec<String> {
        let mut files = Vec::new();
        for p in &self.podcasts {
            files.push(p.file.clone());
            if let Some(cit) = p.citation() {
                files.push(cit.to_string());
            }
            if let Some(audio) = &p.audio {
                files.push(audio.clone());
            }
            if let Some(thumb) = &p.thumbnail {
                files.push(thumb.clone());
            }
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yaml_parse() {
        let yaml = r#"
podcasts:
  - title: "Test Podcast"
    file: content/test.md
    source_url: "https://example.com"
    category: "tech"
    timeline:
      - content/test.bib
      - 26
"#;
        let meta: Metadata = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(meta.podcasts.len(), 1);
        assert_eq!(meta.podcasts[0].title, "Test Podcast");
        assert_eq!(
            meta.podcasts[0].source_url.as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            meta.podcasts[0].timeline,
            vec![
                TimelineItem::Citation("content/test.bib".to_string()),
                TimelineItem::News(26),
            ]
        );
        assert_eq!(meta.podcasts[0].citation(), Some("content/test.bib"));
    }

    #[test]
    fn test_yaml_parse_without_timeline() {
        let meta: Metadata =
            serde_yaml::from_str("podcasts:\n  - title: T\n    file: f.md\n").unwrap();
        assert!(meta.podcasts[0].timeline.is_empty());
        assert_eq!(meta.podcasts[0].citation(), None);
    }

    #[test]
    fn test_referenced_files_includes_all() {
        let meta = Metadata {
            podcasts: vec![Podcast {
                title: "P".into(),
                file: "content/p.md".into(),
                source_url: None,
                category: None,
                thumbnail: Some("assets/image/p.jpg".into()),
                audio: Some("assets/audio/p.mp3".into()),
                timeline: vec![TimelineItem::Citation("content/p.bib".into())],
            }],
        };

        let files = meta.referenced_files();
        assert_eq!(files.len(), 4);
        assert!(files.contains(&"content/p.md".to_string()));
        assert!(files.contains(&"content/p.bib".to_string()));
        assert!(files.contains(&"assets/audio/p.mp3".to_string()));
        assert!(files.contains(&"assets/image/p.jpg".to_string()));
    }
}
