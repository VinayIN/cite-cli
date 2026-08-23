use std::collections::HashMap;
use std::path::{Path, PathBuf};

use libsql::{Builder, Connection, Value, params};
use tracing::info;

use crate::core::CiteError;
use crate::core::cache::{BuildCache, UuidCache};
use crate::core::project::ProjectContext;

pub fn global_db_path() -> PathBuf {
    if let Ok(path) = std::env::var("CITE_DB_PATH") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".cite").join("cite.db")
}

pub struct DbManager {
    conn: Connection,
}

fn get_opt_string(row: &libsql::Row, idx: i32) -> String {
    match row.get_value(idx) {
        Ok(Value::Text(s)) => s,
        _ => String::new(),
    }
}

fn opt_string(row: &libsql::Row, idx: i32) -> Option<String> {
    let s = get_opt_string(row, idx);
    if s.is_empty() { None } else { Some(s) }
}

impl DbManager {
    pub async fn open() -> Result<Self, CiteError> {
        let path = global_db_path();
        Self::open_path(&path).await
    }

    pub async fn open_path(path: &Path) -> Result<Self, CiteError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Builder::new_local(path).build().await?;
        let conn = db.connect()?;
        let mgr = Self { conn };
        mgr.run_migrations().await?;
        Ok(mgr)
    }

    async fn run_migrations(&self) -> Result<(), CiteError> {
        let batch = "
            CREATE TABLE IF NOT EXISTS _schema_version (
                version INTEGER PRIMARY KEY,
                applied_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', CURRENT_TIMESTAMP))
            );
            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                root_path TEXT NOT NULL DEFAULT '',
                language TEXT DEFAULT 'en',
                artist_id TEXT DEFAULT '',
                metadata_file TEXT DEFAULT 'metadata.yml',
                last_synced TEXT
            );
            CREATE TABLE IF NOT EXISTS podcasts (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                file TEXT NOT NULL DEFAULT '',
                source_url TEXT,
                category TEXT,
                thumbnail TEXT,
                audio TEXT,
                citation_file TEXT,
                content TEXT,
                word_count INTEGER DEFAULT 0,
                built_at TEXT
            );
            CREATE TABLE IF NOT EXISTS timeline_entries (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                podcast_id TEXT NOT NULL,
                date TEXT,
                title TEXT NOT NULL DEFAULT '',
                summary TEXT,
                url TEXT,
                entry_type TEXT
            );
            CREATE TABLE IF NOT EXISTS build_history (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                compiler_version REAL NOT NULL,
                built_at TEXT NOT NULL,
                podcast_count INTEGER DEFAULT 0,
                timeline_count INTEGER DEFAULT 0,
                total_words INTEGER DEFAULT 0,
                duration_ms INTEGER DEFAULT 0,
                was_incremental INTEGER DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS deployment_history (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                deployment_id TEXT NOT NULL,
                deployed_at TEXT NOT NULL,
                storage_path TEXT DEFAULT '',
                news_count INTEGER DEFAULT 0,
                timeline_count INTEGER DEFAULT 0,
                asset_count INTEGER DEFAULT 0,
                success INTEGER DEFAULT 1,
                dry_run INTEGER DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS file_cache (
                file_path TEXT NOT NULL,
                project_id TEXT NOT NULL,
                sha256 TEXT NOT NULL DEFAULT '',
                last_modified TEXT,
                PRIMARY KEY (file_path, project_id)
            );
        ";

        for stmt in batch.split(';') {
            let trimmed = stmt.trim();
            if !trimmed.is_empty() {
                self.conn.execute(trimmed, ()).await?;
            }
        }

        if self
            .conn
            .query("SELECT link FROM timeline_entries LIMIT 1", ())
            .await
            .is_err()
        {
            self.conn
                .execute("ALTER TABLE timeline_entries ADD COLUMN link TEXT", ())
                .await?;
        }

        let mut rows = self
            .conn
            .query("SELECT COUNT(*) FROM _schema_version", ())
            .await?;
        let has_version = match rows.next().await? {
            Some(row) => row.get::<i64>(0)? > 0,
            None => false,
        };

        if !has_version {
            self.conn
                .execute("INSERT INTO _schema_version (version) VALUES (1)", ())
                .await?;
        }

        Ok(())
    }

    async fn project_ensure(&self, project_id: &str, name: &str) -> Result<(), CiteError> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO projects (id, name, root_path, last_synced)
                 VALUES (?1, ?2, '', strftime('%Y-%m-%dT%H:%M:%fZ', CURRENT_TIMESTAMP))",
                params![project_id, name],
            )
            .await?;
        Ok(())
    }

    async fn project_update_sync(
        &self,
        project_id: &str,
        ctx: &ProjectContext,
    ) -> Result<(), CiteError> {
        self.conn
            .execute(
                "UPDATE projects SET name = ?1, root_path = ?2, language = ?3, artist_id = ?4,
                        metadata_file = ?5, last_synced = strftime('%Y-%m-%dT%H:%M:%fZ', CURRENT_TIMESTAMP)
                 WHERE id = ?6",
                params![
                    ctx.manifest.project.name.clone(),
                    ctx.root.to_string_lossy().to_string(),
                    ctx.manifest.project.language.clone(),
                    ctx.manifest.project.artist_id.clone(),
                    ctx.manifest.project.metadata_file.clone(),
                    project_id,
                ],
            )
            .await?;
        Ok(())
    }

    pub async fn sync_project(&self, ctx: &ProjectContext) -> Result<(), CiteError> {
        let project_id = ctx.root.to_string_lossy().to_string();
        let name = &ctx.manifest.project.name;

        self.project_ensure(&project_id, name).await?;
        self.project_update_sync(&project_id, ctx).await?;

        self.conn
            .execute(
                "DELETE FROM podcasts WHERE project_id = ?1",
                params![project_id.clone()],
            )
            .await?;
        self.conn
            .execute(
                "DELETE FROM timeline_entries WHERE project_id = ?1",
                params![project_id.clone()],
            )
            .await?;

        let mut uuid_cache = UuidCache::load(&ctx.root);

        for pod in &ctx.metadata.podcasts {
            let content = read_content_file(&ctx.root.join(&pod.file));
            let wc = content
                .as_deref()
                .map(|c| c.split_whitespace().count() as i64)
                .unwrap_or(0);

            let pod_id = uuid_cache.get_or_create(&format!("podcast:{}:{}", project_id, pod.file));

            self.conn
                .execute(
                    "INSERT INTO podcasts (id, project_id, title, file, source_url, category,
                            thumbnail, audio, citation_file, content, word_count)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        pod_id,
                        project_id.clone(),
                        pod.title.clone(),
                        pod.file.clone(),
                        pod.source_url.clone(),
                        pod.category.clone(),
                        pod.thumbnail.clone(),
                        pod.audio.clone(),
                        pod.citation.clone(),
                        content,
                        wc,
                    ],
                )
                .await?;
        }

        for pod in &ctx.metadata.podcasts {
            if let Some(cit) = &pod.citation {
                let bib_path = ctx.root.join(cit);
                if bib_path.exists()
                    && let Ok(raw) = std::fs::read_to_string(&bib_path)
                {
                    let entries = crate::core::compiler::parse_bibtex(&raw);
                    let tl_id =
                        uuid_cache.get_or_create(&format!("timeline:{}:{}", project_id, cit));
                    for entry in &entries {
                        let entry_id =
                            uuid_cache.get_or_create(&format!("entry:{}:{}", tl_id, entry.title));
                        let none_str: Option<String> = None;
                        let tl_id_param = tl_id.clone();
                        let entry_date = entry.date.clone();
                        let entry_title = entry.title.clone();
                        let entry_summary = entry.summary.clone();
                        let entry_url = entry.url.clone();
                        let entry_link = entry.link.clone();
                        self.conn
                                .execute(
                                    "INSERT INTO timeline_entries
                                        (id, project_id, podcast_id, date, title, summary, url, link, entry_type)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                                    params![
                                        entry_id,
                                        project_id.clone(),
                                        tl_id_param,
                                        entry_date,
                                        entry_title,
                                        entry_summary,
                                        entry_url,
                                        entry_link,
                                        none_str,
                                    ],
                                )
                                .await?;
                    }
                }
            }
        }

        uuid_cache.save(&ctx.root);

        info!(
            "Synced {} podcast(s) and timeline entries for '{}'",
            ctx.metadata.podcasts.len(),
            name
        );
        Ok(())
    }

    pub async fn load_cache(&self, project_id: &str) -> Result<Option<BuildCache>, CiteError> {
        let mut rows = self
            .conn
            .query(
                "SELECT file_path, sha256 FROM file_cache WHERE project_id = ?1",
                params![project_id],
            )
            .await?;

        let mut hashes = HashMap::new();
        while let Some(row) = rows.next().await? {
            let path: String = row.get(0)?;
            let hash: String = row.get(1)?;
            hashes.insert(path, hash);
        }

        let cv: f64 = self
            .conn
            .query(
                "SELECT MAX(compiler_version) FROM build_history WHERE project_id = ?1",
                params![project_id],
            )
            .await?
            .next()
            .await?
            .map(|row| row.get::<f64>(0).unwrap_or(0.0))
            .unwrap_or(0.0);

        if hashes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(BuildCache::new(cv, hashes)))
        }
    }

    pub async fn save_cache(
        &self,
        project_id: &str,
        hashes: &HashMap<String, String>,
    ) -> Result<(), CiteError> {
        self.conn
            .execute(
                "DELETE FROM file_cache WHERE project_id = ?1",
                params![project_id],
            )
            .await?;

        let now = chrono::Utc::now().to_rfc3339();

        for (path, hash) in hashes {
            self.conn
                .execute(
                    "INSERT INTO file_cache (file_path, project_id, sha256, last_modified)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![path.clone(), project_id, hash.clone(), now.clone()],
                )
                .await?;
        }

        Ok(())
    }

    pub async fn clear_cache(&self, project_id: &str) -> Result<(), CiteError> {
        self.conn
            .execute(
                "DELETE FROM file_cache WHERE project_id = ?1",
                params![project_id],
            )
            .await?;
        Ok(())
    }

    pub async fn record_build(
        &self,
        record: &super::project::BuildRecord,
    ) -> Result<(), CiteError> {
        let now = chrono::Utc::now().to_rfc3339();
        let id = uuid::Uuid::new_v4().to_string();
        self.conn
            .execute(
                "INSERT INTO build_history
                        (id, project_id, compiler_version, built_at, podcast_count, timeline_count, total_words, duration_ms, was_incremental)
                  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id,
                    record.project_id.clone(),
                    record.compiler_version,
                    now,
                    record.podcast_count,
                    record.timeline_count,
                    record.total_words,
                    record.duration_ms,
                    record.was_incremental as i64,
                ],
            )
            .await?;
        Ok(())
    }

    pub async fn record_deployment(
        &self,
        report: &super::project::DeployReport,
    ) -> Result<(), CiteError> {
        let now = chrono::Utc::now().to_rfc3339();
        let id = uuid::Uuid::new_v4().to_string();
        self.conn
            .execute(
                "INSERT INTO deployment_history
                        (id, project_id, deployment_id, deployed_at, storage_path, news_count, timeline_count, asset_count, success, dry_run)
                  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    report.project_id.clone(),
                    report.deployment_id.clone(),
                    now,
                    report.storage_path.clone(),
                    report.news_count,
                    report.timeline_count,
                    report.asset_count,
                    report.success as i64,
                    report.dry_run as i64,
                ],
            )
            .await?;
        Ok(())
    }

    pub async fn get_podcasts_with_content(
        &self,
        project_id: &str,
    ) -> Result<Vec<super::project::StoredPodcast>, CiteError> {
        let mut rows = self
            .conn
            .query(
                "SELECT title, word_count, category, file, audio, thumbnail
                 FROM podcasts WHERE project_id = ?1 ORDER BY title",
                params![project_id],
            )
            .await?;

        let mut podcasts = Vec::new();
        while let Some(row) = rows.next().await? {
            podcasts.push(super::project::StoredPodcast {
                title: row.get(0)?,
                word_count: row.get(1)?,
                category: get_opt_string(&row, 2),
                file: row.get(3)?,
                has_audio: !get_opt_string(&row, 4).is_empty(),
                has_thumbnail: !get_opt_string(&row, 5).is_empty(),
            });
        }
        Ok(podcasts)
    }

    pub async fn get_timelines(
        &self,
        project_id: &str,
    ) -> Result<Vec<super::project::StoredTimeline>, CiteError> {
        let mut rows = self
            .conn
            .query(
                "SELECT date, title, url, entry_type, link
                 FROM timeline_entries WHERE project_id = ?1
                 ORDER BY date DESC NULLS LAST",
                params![project_id],
            )
            .await?;

        let mut entries = Vec::new();
        while let Some(row) = rows.next().await? {
            let d = get_opt_string(&row, 0);
            let u = get_opt_string(&row, 2);
            let t = get_opt_string(&row, 3);
            let l = get_opt_string(&row, 4);
            entries.push(super::project::StoredTimeline {
                date: if d.is_empty() { None } else { Some(d) },
                title: row.get(1)?,
                url: if u.is_empty() { None } else { Some(u) },
                entry_type: if t.is_empty() { None } else { Some(t) },
                link: if l.is_empty() { None } else { Some(l) },
            });
        }
        Ok(entries)
    }

    pub async fn get_restore_snapshot(
        &self,
        project_id: &str,
    ) -> Result<super::project::RestoredProject, CiteError> {
        use super::project::{RestoredPodcast, RestoredProject, RestoredTimeline};

        let mut rows = self
            .conn
            .query(
                "SELECT name, language, artist_id, metadata_file FROM projects WHERE id = ?1",
                params![project_id],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Err(CiteError::Config(format!(
                "No local record found for project '{project_id}'"
            )));
        };
        let mut snapshot = RestoredProject {
            name: row.get(0)?,
            language: get_opt_string(&row, 1),
            artist_id: get_opt_string(&row, 2),
            metadata_file: get_opt_string(&row, 3),
            podcasts: Vec::new(),
            timelines: Vec::new(),
        };
        if snapshot.language.is_empty() {
            snapshot.language = "en".into();
        }
        if snapshot.metadata_file.is_empty() {
            snapshot.metadata_file = "metadata.yml".into();
        }

        let mut rows = self
            .conn
            .query(
                "SELECT id, title, file, source_url, category, thumbnail, audio, citation_file, content
                 FROM podcasts WHERE project_id = ?1 ORDER BY file",
                params![project_id],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            snapshot.podcasts.push(RestoredPodcast {
                id: row.get(0)?,
                title: row.get(1)?,
                file: get_opt_string(&row, 2),
                source_url: opt_string(&row, 3),
                category: opt_string(&row, 4),
                thumbnail: opt_string(&row, 5),
                audio: opt_string(&row, 6),
                citation_file: opt_string(&row, 7),
                content: opt_string(&row, 8),
            });
        }

        let mut rows = self
            .conn
            .query(
                "SELECT podcast_id, date, title, summary, url, link
                 FROM timeline_entries WHERE project_id = ?1
                 ORDER BY date ASC NULLS LAST",
                params![project_id],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            snapshot.timelines.push(RestoredTimeline {
                podcast_id: row.get(0)?,
                date: opt_string(&row, 1),
                title: get_opt_string(&row, 2),
                summary: opt_string(&row, 3),
                url: opt_string(&row, 4),
                link: opt_string(&row, 5),
            });
        }

        Ok(snapshot)
    }

    pub async fn get_deployment_history(
        &self,
        project_id: &str,
    ) -> Result<Vec<super::project::StoredDeployment>, CiteError> {
        let mut rows = self
            .conn
            .query(
                "SELECT deployment_id, deployed_at, success, news_count, asset_count
                 FROM deployment_history WHERE project_id = ?1
                 ORDER BY deployed_at DESC",
                params![project_id],
            )
            .await?;

        let mut deployments = Vec::new();
        while let Some(row) = rows.next().await? {
            deployments.push(super::project::StoredDeployment {
                deployment_id: row.get(0)?,
                deployed_at: row.get(1)?,
                success: row.get::<i64>(2)? != 0,
                news_count: row.get(3)?,
                asset_count: row.get(4)?,
            });
        }
        Ok(deployments)
    }

    pub async fn get_build_history(
        &self,
        project_id: &str,
    ) -> Result<Vec<super::project::StoredBuild>, CiteError> {
        let mut rows = self
            .conn
            .query(
                "SELECT podcast_count, timeline_count, total_words, duration_ms, was_incremental, built_at
                 FROM build_history WHERE project_id = ?1
                 ORDER BY built_at DESC
                 LIMIT 50",
                params![project_id],
            )
            .await?;

        let mut builds = Vec::new();
        while let Some(row) = rows.next().await? {
            builds.push(super::project::StoredBuild {
                podcast_count: row.get(0)?,
                timeline_count: row.get(1)?,
                total_words: row.get(2)?,
                duration_ms: row.get(3)?,
                was_incremental: row.get::<i64>(4)? != 0,
                built_at: get_opt_string(&row, 5),
            });
        }
        Ok(builds)
    }

    pub async fn get_project_stats(
        &self,
        project_id: &str,
    ) -> Result<super::project::ProjectStats, CiteError> {
        let podcast_count: i64 = self
            .conn
            .query(
                "SELECT COUNT(*) FROM podcasts WHERE project_id = ?1",
                params![project_id],
            )
            .await?
            .next()
            .await?
            .map(|row| row.get::<i64>(0).unwrap_or(0))
            .unwrap_or(0);

        let timeline_count: i64 = self
            .conn
            .query(
                "SELECT COUNT(*) FROM timeline_entries WHERE project_id = ?1",
                params![project_id],
            )
            .await?
            .next()
            .await?
            .map(|row| row.get::<i64>(0).unwrap_or(0))
            .unwrap_or(0);

        let total_words: i64 = self
            .conn
            .query(
                "SELECT COALESCE(SUM(word_count), 0) FROM podcasts WHERE project_id = ?1",
                params![project_id],
            )
            .await?
            .next()
            .await?
            .map(|row| row.get::<i64>(0).unwrap_or(0))
            .unwrap_or(0);

        let build_count: i64 = self
            .conn
            .query(
                "SELECT COUNT(*) FROM build_history WHERE project_id = ?1",
                params![project_id],
            )
            .await?
            .next()
            .await?
            .map(|row| row.get::<i64>(0).unwrap_or(0))
            .unwrap_or(0);

        let last_built: Option<String> = self
            .conn
            .query(
                "SELECT built_at FROM build_history WHERE project_id = ?1 ORDER BY built_at DESC LIMIT 1",
                params![project_id],
            )
            .await?
            .next()
            .await?
            .map(|row| {
                let s: String = row.get(0).unwrap_or_default();
                s
            })
            .filter(|s| !s.is_empty());

        let deployment_count: i64 = self
            .conn
            .query(
                "SELECT COUNT(*) FROM deployment_history WHERE project_id = ?1 AND success = 1",
                params![project_id],
            )
            .await?
            .next()
            .await?
            .map(|row| row.get::<i64>(0).unwrap_or(0))
            .unwrap_or(0);

        let last_deployed: Option<String> = self
            .conn
            .query(
                "SELECT deployed_at FROM deployment_history WHERE project_id = ?1 AND success = 1 ORDER BY deployed_at DESC LIMIT 1",
                params![project_id],
            )
            .await?
            .next()
            .await?
            .map(|row| {
                let s: String = row.get(0).unwrap_or_default();
                s
            })
            .filter(|s| !s.is_empty());

        Ok(super::project::ProjectStats {
            podcast_count,
            timeline_count,
            total_words,
            build_count,
            last_built,
            deployment_count,
            last_deployed,
        })
    }

    pub async fn list_db_projects(&self) -> Result<Vec<(String, String)>, CiteError> {
        let mut rows = self
            .conn
            .query("SELECT name, id FROM projects ORDER BY name", ())
            .await?;
        let mut projects = Vec::new();
        while let Some(row) = rows.next().await? {
            let name: String = row.get(0)?;
            let id: String = row.get(1)?;
            projects.push((name, id));
        }
        Ok(projects)
    }

    pub async fn get_all_stats(&self) -> Result<super::project::AllStats, CiteError> {
        let project_count: i64 = self
            .conn
            .query("SELECT COUNT(*) FROM projects", ())
            .await?
            .next()
            .await?
            .map(|row| row.get::<i64>(0).unwrap_or(0))
            .unwrap_or(0);

        let total_podcasts: i64 = self
            .conn
            .query("SELECT COUNT(*) FROM podcasts", ())
            .await?
            .next()
            .await?
            .map(|row| row.get::<i64>(0).unwrap_or(0))
            .unwrap_or(0);

        let total_timelines: i64 = self
            .conn
            .query("SELECT COUNT(*) FROM timeline_entries", ())
            .await?
            .next()
            .await?
            .map(|row| row.get::<i64>(0).unwrap_or(0))
            .unwrap_or(0);

        let total_words: i64 = self
            .conn
            .query("SELECT COALESCE(SUM(word_count), 0) FROM podcasts", ())
            .await?
            .next()
            .await?
            .map(|row| row.get::<i64>(0).unwrap_or(0))
            .unwrap_or(0);

        let total_builds: i64 = self
            .conn
            .query("SELECT COUNT(*) FROM build_history", ())
            .await?
            .next()
            .await?
            .map(|row| row.get::<i64>(0).unwrap_or(0))
            .unwrap_or(0);

        Ok(super::project::AllStats {
            project_count,
            total_podcasts,
            total_timelines,
            total_words,
            total_builds,
        })
    }
}

fn read_content_file(path: &Path) -> Option<String> {
    if path.exists() && path.is_file() {
        std::fs::read_to_string(path).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_migration_and_queries() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = DbManager::open_path(&db_path).await.unwrap();

        db.conn
            .execute(
                "INSERT INTO projects (id, name) VALUES ('proj1', 'Test Project')",
                (),
            )
            .await
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO podcasts (id, project_id, title, file, word_count)
                 VALUES ('p1', 'proj1', 'Test Podcast', 'test.md', 100)",
                (),
            )
            .await
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO timeline_entries (id, project_id, podcast_id, date, title)
                 VALUES ('t1', 'proj1', 'p1', '2005-03', 'Test Entry')",
                (),
            )
            .await
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO build_history (id, project_id, compiler_version, built_at, podcast_count, timeline_count, total_words, duration_ms, was_incremental)
                 VALUES ('b1', 'proj1', 1.0, '2026-07-29T15:00:00.000Z', 1, 1, 100, 50, 0)",
                (),
            )
            .await
            .unwrap();

        let stats = db.get_project_stats("proj1").await.unwrap();
        assert_eq!(stats.podcast_count, 1);
        assert_eq!(stats.timeline_count, 1);
        assert_eq!(stats.total_words, 100);
        assert_eq!(stats.build_count, 1);
        assert_eq!(stats.deployment_count, 0);
        assert!(stats.last_built.is_some());

        let all = db.get_all_stats().await.unwrap();
        assert_eq!(all.project_count, 1);
        assert_eq!(all.total_podcasts, 1);
        assert_eq!(all.total_timelines, 1);
        assert_eq!(all.total_words, 100);
        assert_eq!(all.total_builds, 1);

        let pods = db.get_podcasts_with_content("proj1").await.unwrap();
        assert_eq!(pods.len(), 1);
        assert_eq!(pods[0].title, "Test Podcast");

        let timelines = db.get_timelines("proj1").await.unwrap();
        assert_eq!(timelines.len(), 1);

        let builds = db.get_build_history("proj1").await.unwrap();
        assert_eq!(builds.len(), 1);
    }

    #[tokio::test]
    async fn test_restore_snapshot_and_link_column() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = DbManager::open_path(&db_path).await.unwrap();

        db.conn
            .execute(
                "INSERT INTO projects (id, name, language, artist_id)
                 VALUES ('proj1', 'Restore Me', 'en', 'artist-uuid')",
                (),
            )
            .await
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO podcasts (id, project_id, title, file, source_url, content, word_count)
                 VALUES ('pod1', 'proj1', 'Episode', 'content/ep.md', 'https://example.com', '# Episode', 2)",
                (),
            )
            .await
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO timeline_entries (id, project_id, podcast_id, date, title, summary, url, link)
                 VALUES ('t1', 'proj1', 'pod1', '2024-02', 'Event', 'Summary text', 'https://example.com/a', 'https://example.com/news/b')",
                (),
            )
            .await
            .unwrap();

        let timelines = db.get_timelines("proj1").await.unwrap();
        assert_eq!(
            timelines[0].link.as_deref(),
            Some("https://example.com/news/b")
        );

        let snap = db.get_restore_snapshot("proj1").await.unwrap();
        assert_eq!(snap.name, "Restore Me");
        assert_eq!(snap.language, "en");
        assert_eq!(snap.artist_id, "artist-uuid");
        assert_eq!(snap.podcasts.len(), 1);
        assert_eq!(snap.podcasts[0].content.as_deref(), Some("# Episode"));
        assert_eq!(
            snap.podcasts[0].source_url.as_deref(),
            Some("https://example.com")
        );
        assert_eq!(snap.timelines.len(), 1);
        assert_eq!(snap.timelines[0].title, "Event");
        assert_eq!(snap.timelines[0].summary.as_deref(), Some("Summary text"));
        assert_eq!(
            snap.timelines[0].link.as_deref(),
            Some("https://example.com/news/b")
        );

        assert!(db.get_restore_snapshot("missing").await.is_err());
    }
}
