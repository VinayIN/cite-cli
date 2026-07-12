use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation: Option<String>,
}

impl Default for Podcast {
    fn default() -> Self {
        Self {
            title: String::new(),
            file: String::new(),
            source_url: None,
            category: None,
            thumbnail: None,
            audio: None,
            citation: None,
        }
    }
}

fn get_uuid() -> String {
    Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TimelineEntry {
    pub id: String,
    pub date: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub url: Option<String>,
}

impl Default for TimelineEntry {
    fn default() -> Self {
        Self {
            id: get_uuid(),
            date: None,
            title: String::new(),
            summary: None,
            url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Metadata {
    pub podcasts: Vec<Podcast>,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            podcasts: vec![Podcast::default()],
        }
    }
}

impl Metadata {
    pub fn referenced_files(&self) -> Vec<String> {
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
  - title: "Test Podcast"
    file: content/test.md
    source_url: "https://example.com"
    category: "tech"
"#;
        let meta: Metadata = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(meta.podcasts.len(), 1);
        assert_eq!(meta.podcasts[0].title, "Test Podcast");
        assert_eq!(meta.podcasts[0].file, "content/test.md");
        assert_eq!(
            meta.podcasts[0].source_url.as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn test_referenced_files_includes_all() {
        let meta = Metadata {
            podcasts: vec![Podcast {
                title: "P".into(),
                file: "content/p.md".into(),
                source_url: None,
                category: None,
                thumbnail: None,
                audio: Some("assets/audio/p.mp3".into()),
                citation: Some("content/p.bib".into()),
                ..Default::default()
            }],
        };

        let files = meta.referenced_files();
        assert_eq!(files.len(), 3);
        assert!(files.contains(&"content/p.md".to_string()));
        assert!(files.contains(&"content/p.bib".to_string()));
        assert!(files.contains(&"assets/audio/p.mp3".to_string()));
    }
}
