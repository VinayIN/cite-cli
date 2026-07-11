use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Metadata {
    #[serde(default)]
    pub podcasts: Vec<Podcast>,
}

impl Metadata {
    pub fn content_files(&self) -> Vec<String> {
        let mut files = Vec::new();
        for p in &self.podcasts {
            files.push(p.file.clone());
            if let Some(cit) = &p.citation {
                files.push(cit.clone());
            }
            if let Some(audio) = &p.audio {
                files.push(audio.clone());
            }
        }
        files
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Podcast {
    pub id: Option<String>,
    pub title: String,
    pub file: String,
    pub source_url: Option<String>,
    pub category: Option<String>,
    pub thumbnail: Option<String>,
    pub audio: Option<String>,
    pub citation: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub date: String,
    pub id: String,
    pub title: String,
    pub summary: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timeline {
    pub id: String,
    pub entries: Vec<TimelineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBundle {
    pub compiler_version: String,
    pub project: String,
    pub artist_id: String,
    pub podcasts: Vec<Podcast>,
    #[serde(default)]
    pub timelines: Vec<Timeline>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_yaml_parse() {
        let yaml = r#"
podcasts:
  - id: "abc"
    title: "Test Podcast"
    file: content/test.md
    source_url: "https://example.com"
    category: "tech"
"#;
        let meta: Metadata = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(meta.podcasts.len(), 1);
        assert_eq!(meta.podcasts[0].id.as_deref(), Some("abc"));
        assert_eq!(meta.podcasts[0].title, "Test Podcast");
        assert_eq!(meta.podcasts[0].file, "content/test.md");
        assert_eq!(
            meta.podcasts[0].source_url.as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn test_content_files_includes_all() {
        let id = Uuid::new_v4().to_string();
        let meta = Metadata {
            podcasts: vec![Podcast {
                id: Some(id.clone()),
                title: "P".into(),
                file: "content/p.md".into(),
                source_url: None,
                category: None,
                thumbnail: None,
                audio: Some("assets/audio/p.mp3".into()),
                citation: Some("content/p.bib".into()),
                content: None,
            }],
        };
        assert_eq!(meta.podcasts[0].id, Some(id));

        let files = meta.content_files();
        assert_eq!(files.len(), 3);
        assert!(files.contains(&"content/p.md".to_string()));
        assert!(files.contains(&"content/p.bib".to_string()));
        assert!(files.contains(&"assets/audio/p.mp3".to_string()));
    }
}
