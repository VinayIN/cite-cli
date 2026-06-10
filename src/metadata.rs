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
