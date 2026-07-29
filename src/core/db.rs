use std::collections::HashMap;
use std::path::{Path, PathBuf};

use duckdb::{Connection, params};
use tracing::info;

use crate::core::CiteError;
use crate::core::cache::{BuildCache, UuidCache};
use crate::core::project::ProjectContext;

pub fn global_db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".cite").join("cite.db")
}

pub struct DbManager {
    conn: Connection,
}

impl DbManager {
    pub fn open() -> Result<Self, CiteError> {
        let path = global_db_path();
        Self::open_path(&path)
    }

    pub fn open_path(path: &Path) -> Result<Self, CiteError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        let mgr = Self { conn };
        mgr.run_migrations()?;
        Ok(mgr)
    }

    fn run_migrations(&self) -> Result<(), CiteError> {
        self.conn.execute_batch(
            "
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

            CREATE INDEX IF NOT EXISTS idx_podcasts_project ON podcasts(project_id);
            CREATE INDEX IF NOT EXISTS idx_timeline_project ON timeline_entries(project_id);
            CREATE INDEX IF NOT EXISTS idx_build_project ON build_history(project_id);
            CREATE INDEX IF NOT EXISTS idx_deploy_project ON deployment_history(project_id);
            CREATE INDEX IF NOT EXISTS idx_filecache_project ON file_cache(project_id);
            ",
        )?;

        if self
            .conn
            .prepare("SELECT COUNT(*) FROM _schema_version")?
            .query_map([], |_| Ok(()))?
            .next()
            .is_none()
        {
            self.conn
                .execute("INSERT INTO _schema_version (version) VALUES (1)", [])?;
        }

        Ok(())
    }

    fn project_ensure(&self, project_id: &str, name: &str) -> Result<(), CiteError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO projects (id, name, root_path, last_synced)
             VALUES (?1, ?2, '', strftime('%Y-%m-%dT%H:%M:%fZ', CURRENT_TIMESTAMP))",
            params![project_id, name],
        )?;
        Ok(())
    }

    fn project_update_sync(&self, project_id: &str, ctx: &ProjectContext) -> Result<(), CiteError> {
        self.conn.execute(
            "UPDATE projects SET name = ?1, root_path = ?2, language = ?3, artist_id = ?4,
                    metadata_file = ?5, last_synced = strftime('%Y-%m-%dT%H:%M:%fZ', CURRENT_TIMESTAMP)
             WHERE id = ?6",
            params![
                ctx.manifest.project.name,
                ctx.root.to_string_lossy().to_string(),
                ctx.manifest.project.language,
                ctx.manifest.project.artist_id,
                ctx.manifest.project.metadata_file,
                project_id,
            ],
        )?;
        Ok(())
    }

    pub fn sync_project(&self, ctx: &ProjectContext) -> Result<(), CiteError> {
        let project_id = ctx.root.to_string_lossy().to_string();
        let name = &ctx.manifest.project.name;

        self.project_ensure(&project_id, name)?;
        self.project_update_sync(&project_id, ctx)?;

        self.conn.execute(
            "DELETE FROM podcasts WHERE project_id = ?1",
            params![project_id],
        )?;
        self.conn.execute(
            "DELETE FROM timeline_entries WHERE project_id = ?1",
            params![project_id],
        )?;

        // Use persistent UUIDs matching the compiler's scheme
        let mut uuid_cache = UuidCache::load(&ctx.root);

        for pod in &ctx.metadata.podcasts {
            let content = read_content_file(&ctx.root.join(&pod.file));
            let wc = content
                .as_deref()
                .map(|c| c.split_whitespace().count() as i64)
                .unwrap_or(0);

            let pod_id = uuid_cache.get_or_create(&format!("podcast:{}:{}", project_id, pod.file));

            self.conn.execute(
                "INSERT INTO podcasts (id, project_id, title, file, source_url, category,
                        thumbnail, audio, citation_file, content, word_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    pod_id,
                    project_id,
                    pod.title,
                    pod.file,
                    pod.source_url,
                    pod.category,
                    pod.thumbnail,
                    pod.audio,
                    pod.citation,
                    content,
                    wc,
                ],
            )?;
        }

        for pod in &ctx.metadata.podcasts {
            if let Some(cit) = &pod.citation {
                let bib_path = ctx.root.join(cit);
                if bib_path.exists()
                    && let Ok(raw) = std::fs::read_to_string(&bib_path) {
                        let entries = crate::core::compiler::parse_bibtex(&raw);
                        let tl_id = uuid_cache.get_or_create(&format!("timeline:{}:{}", project_id, cit));
                        for entry in &entries {
                            let entry_id = uuid_cache.get_or_create(
                                &format!("entry:{}:{}", tl_id, entry.title),
                            );
                            self.conn.execute(
                                "INSERT INTO timeline_entries
                                        (id, project_id, podcast_id, date, title, summary, url, entry_type)
                                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                                params![
                                    entry_id,
                                    project_id,
                                    tl_id,
                                    entry.date,
                                    entry.title,
                                    entry.summary,
                                    entry.url,
                                    Option::<String>::None,
                                ],
                            )?;
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

    pub fn load_cache(&self, project_id: &str) -> Result<Option<BuildCache>, CiteError> {
        let mut stmt = self
            .conn
            .prepare("SELECT file_path, sha256 FROM file_cache WHERE project_id = ?1")?;

        let rows = stmt.query_map(params![project_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut hashes = HashMap::new();
        for row in rows {
            let (path, hash) = row?;
            hashes.insert(path, hash);
        }

        let cv: f64 = self
            .conn
            .query_row(
                "SELECT MAX(compiler_version) FROM build_history WHERE project_id = ?1",
                params![project_id],
                |row| row.get(0),
            )
            .unwrap_or(0.0);

        if hashes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(BuildCache::new(cv, hashes)))
        }
    }

    pub fn save_cache(
        &self,
        project_id: &str,
        _compiler_version: f64,
        hashes: &HashMap<String, String>,
    ) -> Result<(), CiteError> {
        self.conn.execute(
            "DELETE FROM file_cache WHERE project_id = ?1",
            params![project_id],
        )?;

        let now = chrono::Utc::now().to_rfc3339();
        let mut stmt = self.conn.prepare(
            "INSERT INTO file_cache (file_path, project_id, sha256, last_modified)
             VALUES (?1, ?2, ?3, ?4)",
        )?;

        for (path, hash) in hashes {
            stmt.execute(params![path, project_id, hash, now])?;
        }

        Ok(())
    }

    pub fn clear_cache(&self, project_id: &str) -> Result<(), CiteError> {
        self.conn.execute(
            "DELETE FROM file_cache WHERE project_id = ?1",
            params![project_id],
        )?;
        Ok(())
    }

    // ── Build history ──

    #[allow(clippy::too_many_arguments)]
    pub fn record_build(
        &self,
        project_id: &str,
        compiler_version: f64,
        podcast_count: i64,
        timeline_count: i64,
        total_words: i64,
        duration_ms: i64,
        was_incremental: bool,
    ) -> Result<(), CiteError> {
        let now = chrono::Utc::now().to_rfc3339();
        let id = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO build_history
                    (id, project_id, compiler_version, built_at, podcast_count, timeline_count, total_words, duration_ms, was_incremental)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                project_id,
                compiler_version,
                now,
                podcast_count,
                timeline_count,
                total_words,
                duration_ms,
                was_incremental as i64,
            ],
        )?;
        Ok(())
    }

    // ── Deployment history ──

    #[allow(clippy::too_many_arguments)]
    pub fn record_deployment(
        &self,
        project_id: &str,
        deployment_id: &str,
        storage_path: &str,
        news_count: i64,
        timeline_count: i64,
        asset_count: i64,
        success: bool,
        dry_run: bool,
    ) -> Result<(), CiteError> {
        let now = chrono::Utc::now().to_rfc3339();
        let id = uuid::Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO deployment_history
                    (id, project_id, deployment_id, deployed_at, storage_path, news_count, timeline_count, asset_count, success, dry_run)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                project_id,
                deployment_id,
                now,
                storage_path,
                news_count,
                timeline_count,
                asset_count,
                success as i64,
                dry_run as i64,
            ],
        )?;
        Ok(())
    }

    // ── Browse / List ──

    pub fn get_podcasts_with_content(
        &self,
        project_id: &str,
    ) -> Result<Vec<super::project::StoredPodcast>, CiteError> {
        let mut stmt = self.conn.prepare(
            "SELECT title, word_count FROM podcasts WHERE project_id = ?1 ORDER BY title",
        )?;

        let rows = stmt.query_map(params![project_id], |row| {
            Ok(super::project::StoredPodcast {
                title: row.get(0)?,
                word_count: row.get(1)?,
            })
        })?;

        let mut podcasts = Vec::new();
        for row in rows {
            podcasts.push(row?);
        }
        Ok(podcasts)
    }

    pub fn get_timelines(
        &self,
        project_id: &str,
    ) -> Result<Vec<super::project::StoredTimeline>, CiteError> {
        let mut stmt = self.conn.prepare(
            "SELECT date, title FROM timeline_entries WHERE project_id = ?1 ORDER BY date DESC NULLS LAST",
        )?;

        let rows = stmt.query_map(params![project_id], |row| {
            Ok(super::project::StoredTimeline {
                date: row.get(0)?,
                title: row.get(1)?,
            })
        })?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    pub fn get_deployment_history(
        &self,
        project_id: &str,
    ) -> Result<Vec<super::project::StoredDeployment>, CiteError> {
        let mut stmt = self.conn.prepare(
            "SELECT deployment_id, deployed_at, success FROM deployment_history WHERE project_id = ?1 ORDER BY deployed_at DESC",
        )?;

        let rows = stmt.query_map(params![project_id], |row| {
            Ok(super::project::StoredDeployment {
                deployment_id: row.get(0)?,
                deployed_at: row.get(1)?,
                success: row.get::<_, i64>(2)? != 0,
            })
        })?;

        let mut deployments = Vec::new();
        for row in rows {
            deployments.push(row?);
        }
        Ok(deployments)
    }

    pub fn get_build_history(
        &self,
        project_id: &str,
    ) -> Result<Vec<super::project::StoredBuild>, CiteError> {
        let mut stmt = self.conn.prepare(
            "SELECT podcast_count, timeline_count, total_words, duration_ms, was_incremental
             FROM build_history WHERE project_id = ?1
             ORDER BY built_at DESC
             LIMIT 50",
        )?;

        let rows = stmt.query_map(params![project_id], |row| {
            Ok(super::project::StoredBuild {
                podcast_count: row.get(0)?,
                timeline_count: row.get(1)?,
                total_words: row.get(2)?,
                duration_ms: row.get(3)?,
                was_incremental: row.get::<_, i64>(4)? != 0,
            })
        })?;

        let mut builds = Vec::new();
        for row in rows {
            builds.push(row?);
        }
        Ok(builds)
    }

    // ── Stats / Analytics ──

    pub fn get_project_stats(
        &self,
        project_id: &str,
    ) -> Result<super::project::ProjectStats, CiteError> {
        let podcast_count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM podcasts WHERE project_id = ?1",
                params![project_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let timeline_count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM timeline_entries WHERE project_id = ?1",
                params![project_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let total_words: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(word_count), 0) FROM podcasts WHERE project_id = ?1",
                params![project_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let build_count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM build_history WHERE project_id = ?1",
                params![project_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let last_built: Option<String> = self
            .conn
            .query_row(
                "SELECT built_at FROM build_history WHERE project_id = ?1 ORDER BY built_at DESC LIMIT 1",
                params![project_id],
                |row| row.get(0),
            )
            .ok();

        let deployment_count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM deployment_history WHERE project_id = ?1 AND success = 1",
                params![project_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let last_deployed: Option<String> = self
            .conn
            .query_row(
                "SELECT deployed_at FROM deployment_history WHERE project_id = ?1 AND success = 1 ORDER BY deployed_at DESC LIMIT 1",
                params![project_id],
                |row| row.get(0),
            )
            .ok();

        let mut stmt = self.conn.prepare(
            "SELECT strftime('%Y-%m', built_at::TIMESTAMP) AS month, COUNT(*)
             FROM build_history WHERE project_id = ?1 AND built_at IS NOT NULL
             GROUP BY month ORDER BY month DESC LIMIT 12",
        )?;
        let rows = stmt.query_map(params![project_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut podcasts_by_month = Vec::new();
        for row in rows {
            podcasts_by_month.push(row?);
        }

        let mut stmt = self.conn.prepare(
            "SELECT CAST(CAST(SUBSTR(date, 1, 3) AS INTEGER) / 10 AS INTEGER) * 10 || 's' AS decade, COUNT(*)
             FROM timeline_entries WHERE project_id = ?1 AND date IS NOT NULL AND date != ''
             GROUP BY decade ORDER BY decade",
        )?;
        let rows = stmt.query_map(params![project_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut citations_by_decade = Vec::new();
        for row in rows {
            citations_by_decade.push(row?);
        }

        Ok(super::project::ProjectStats {
            podcast_count,
            timeline_count,
            total_words,
            build_count,
            last_built,
            deployment_count,
            last_deployed,
            podcasts_by_month,
            citations_by_decade,
        })
    }

    pub fn get_all_stats(&self) -> Result<super::project::AllStats, CiteError> {
        let project_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
            .unwrap_or(0);

        let total_podcasts: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM podcasts", [], |row| row.get(0))
            .unwrap_or(0);

        let total_timelines: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM timeline_entries", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);

        let total_words: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(word_count), 0) FROM podcasts",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let total_builds: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM build_history", [], |row| row.get(0))
            .unwrap_or(0);

        let mut stmt = self.conn.prepare(
            "SELECT p.name, COUNT(po.id) AS pods, COALESCE(SUM(po.word_count), 0) AS words
             FROM projects p
             LEFT JOIN podcasts po ON po.project_id = p.id
             GROUP BY p.id, p.name
             ORDER BY words DESC
             LIMIT 10",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut top_projects = Vec::new();
        for row in rows {
            top_projects.push(row?);
        }

        Ok(super::project::AllStats {
            project_count,
            total_podcasts,
            total_timelines,
            total_words,
            total_builds,
            top_projects,
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

    #[test]
    fn test_migration_and_queries() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = DbManager::open_path(&db_path).unwrap();

        // Insert test data
        db.conn.execute_batch("
            INSERT INTO projects (id, name) VALUES ('proj1', 'Test Project');
            INSERT INTO podcasts (id, project_id, title, file, word_count)
                VALUES ('p1', 'proj1', 'Test Podcast', 'test.md', 100);
            INSERT INTO timeline_entries (id, project_id, podcast_id, date, title)
                VALUES ('t1', 'proj1', 'p1', '2005-03', 'Test Entry');
            INSERT INTO build_history (id, project_id, compiler_version, built_at, podcast_count, timeline_count, total_words, duration_ms, was_incremental)
                VALUES ('b1', 'proj1', 1.0, '2026-07-29T15:00:00.000Z', 1, 1, 100, 50, 0);
        ").unwrap();

        // Test get_project_stats
        let stats = db.get_project_stats("proj1").unwrap();
        assert_eq!(stats.podcast_count, 1);
        assert_eq!(stats.timeline_count, 1);
        assert_eq!(stats.total_words, 100);
        assert_eq!(stats.build_count, 1);
        assert_eq!(stats.deployment_count, 0);
        assert!(stats.last_built.is_some());

        // Test get_all_stats
        let all = db.get_all_stats().unwrap();
        assert_eq!(all.project_count, 1);
        assert_eq!(all.total_podcasts, 1);
        assert_eq!(all.total_timelines, 1);
        assert_eq!(all.total_words, 100);
        assert_eq!(all.total_builds, 1);

        // Test get_podcasts_with_content
        let pods = db.get_podcasts_with_content("proj1").unwrap();
        assert_eq!(pods.len(), 1);
        assert_eq!(pods[0].title, "Test Podcast");

        // Test get_timelines
        let timelines = db.get_timelines("proj1").unwrap();
        assert_eq!(timelines.len(), 1);

        // Test get_build_history
        let builds = db.get_build_history("proj1").unwrap();
        assert_eq!(builds.len(), 1);
    }
}


