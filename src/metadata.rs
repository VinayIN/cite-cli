use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Podcast {
    #[serde(default = "get_uuid")]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing)]
    pub file: String,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub audio: Option<String>,
    #[serde(default)]
    pub citation: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
}

fn get_uuid() -> String {
    Uuid::new_v4().to_string()
}

impl Default for Podcast {
    fn default() -> Self {
        Self {
            id: get_uuid(),
            title: String::new(),
            file: String::new(),
            source_url: None,
            category: None,
            thumbnail: None,
            audio: None,
            citation: None,
            content: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    #[serde(default)]
    pub date: String,
    #[serde(default = "get_uuid")]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

impl Default for TimelineEntry {
    fn default() -> Self {
        Self {
            date: String::new(),
            id: Uuid::new_v4().to_string(),
            title: String::new(),
            summary: None,
            url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timeline {
    #[serde(default = "get_uuid")]
    pub id: String,
    #[serde(default)]
    pub entries: Vec<TimelineEntry>,
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            id: get_uuid(),
            entries: vec![TimelineEntry::default()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBundle {
    #[serde(default = "get_compiler_version")]
    pub compiler_version: f64,
    #[serde(default)]
    pub project: String,
    #[serde(default = "get_uuid")]
    pub artist_id: String,
    #[serde(default)]
    pub podcasts: Vec<Podcast>,
    #[serde(default)]
    pub timelines: Vec<Timeline>,
}

fn get_compiler_version() -> f64 {
    0.0
}

impl Default for ContentBundle {
    fn default() -> Self {
        Self {
            compiler_version: get_compiler_version(),
            project: String::new(),
            artist_id: get_uuid(),
            podcasts: vec![Podcast::default()],
            timelines: vec![Timeline::default()],
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(meta.podcasts[0].id, "abc");
        assert_eq!(meta.podcasts[0].title, "Test Podcast");
        assert_eq!(meta.podcasts[0].file, "content/test.md");
        assert_eq!(
            meta.podcasts[0].source_url.as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn test_content_files_includes_all() {
        let meta = Metadata {
            podcasts: vec![Podcast {
                id: "abc".into(),
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

        let files = meta.content_files();
        assert_eq!(files.len(), 3);
        assert!(files.contains(&"content/p.md".to_string()));
        assert!(files.contains(&"content/p.bib".to_string()));
        assert!(files.contains(&"assets/audio/p.mp3".to_string()));
    }
}
