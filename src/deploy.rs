use crate::error::CiteError;
use crate::project::ProjectContext;
use colored::Colorize;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::path::PathBuf;
use tracing::instrument;
use uuid::Uuid;

const STORAGE_BUCKET: &str = "assets";

#[derive(Debug, Clone)]
struct DeployContext<'a> {
    client: &'a reqwest::Client,
    base_url: &'a str,
    service_key: &'a str,
    ctx: &'a ProjectContext,
    artist_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeploymentRecord {
    deployment_id: String,
    storage_path: String,
    news_ids: Vec<i64>,
    timeline_ids: Vec<i64>,
}

#[derive(Debug)]
struct DeployedPodcast {
    news_id: i64,
    timeline_ids: Vec<i64>,
}

fn with_auth(builder: RequestBuilder, service_key: &str) -> RequestBuilder {
    builder
        .header("apikey", service_key)
        .header("Authorization", format!("Bearer {service_key}"))
        .header("Content-Type", "application/json")
}

fn encode_url(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

#[instrument(skip(ctx), fields(project = %ctx.manifest.project.name, dry_run))]
pub async fn deploy(ctx: &ProjectContext, dry_run: bool) -> Result<(), CiteError> {
    let backend = ctx
        .manifest
        .backend
        .as_ref()
        .ok_or_else(|| CiteError::Deploy("No [backend] section in cite.toml".to_string()))?;

    let service_key = resolve_service_key(backend)?;
    let bundle_path = ctx.build_dir().join("content.json");
    if !bundle_path.exists() {
        return Err(CiteError::Deploy(
            "No build artifact found. Run 'cite-cli build' first.".to_string(),
        ));
    }
    let bundle_str = std::fs::read_to_string(&bundle_path)?;
    let bundle: Value = serde_json::from_str(&bundle_str)?;
    let deployment_id = Uuid::new_v4().to_string();
    let project_slug = bundle
        .get("project")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let podcasts = bundle
        .get("podcasts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let timelines = bundle
        .get("timelines")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let artist_id = bundle
        .get("artist_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    eprintln!(
        "{}",
        format!("Deploying with deployment_id: {deployment_id}")
            .cyan()
            .bold()
    );

    if dry_run {
        eprintln!("{}", "  DRY RUN - no data will be sent".yellow().bold());
        eprintln!("{}", format!("  Podcast items: {}", podcasts.len()).cyan());
        eprintln!(
            "{}",
            format!("  Timeline groups: {}", timelines.len()).cyan()
        );
        if !artist_id.is_empty() {
            eprintln!("{}", format!("  Artist ID: {artist_id}").cyan());
        }
        return Ok(());
    }

    let artist_id = Uuid::parse_str(&artist_id).map_err(|_| {
        CiteError::Deploy("artist_id in content.json must be a valid UUID".to_string())
    })?;
    let client = reqwest::Client::new();
    let base_url = backend.staging_url.trim_end_matches('/');
    let storage_path = format!("{project_slug}/{deployment_id}.json");
    let bundle_bytes = serde_json::to_vec_pretty(&bundle)?;
    let public_bundle_url = upload_bytes(
        &client,
        base_url,
        &service_key,
        &storage_path,
        &bundle_bytes,
        "application/json",
    )
    .await?;
    eprintln!("  Uploaded bundle to {}", public_bundle_url.cyan());

    let mut record = DeploymentRecord {
        deployment_id: deployment_id.clone(),
        storage_path: storage_path.clone(),
        news_ids: Vec::new(),
        timeline_ids: Vec::new(),
    };

    let dctx = DeployContext {
        client: &client,
        base_url,
        service_key: &service_key,
        ctx,
        artist_id,
    };

    for pod in &podcasts {
        let deployed = deploy_podcast(&dctx, pod, &timelines).await?;
        record.news_ids.push(deployed.news_id);
        record.timeline_ids.extend(deployed.timeline_ids);
    }

    persist_deployment_record(ctx, &record).await?;

    eprintln!(
        "{}",
        format!("Deployment complete (id: {deployment_id})")
            .green()
            .bold()
    );

    Ok(())
}

async fn deploy_podcast(
    dctx: &DeployContext<'_>,
    podcast: &Value,
    timeline_groups: &[Value],
) -> Result<DeployedPodcast, CiteError> {
    let title = podcast
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled");
    let podcast_id = podcast
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(title);
    let content = podcast.get("content").and_then(|v| v.as_str());
    let category_id =
        ensure_category_id(dctx, podcast.get("category").and_then(|v| v.as_str())).await?;

    let fallback_url = format!("cite://podcasts/{}", podcast_id);
    let url_id = ensure_url_id(
        dctx,
        podcast.get("source_url").and_then(|v| v.as_str()),
        &fallback_url,
        content.map(word_count),
    )
    .await?;

    let thumbnail_url = upload_optional_asset(
        dctx,
        podcast_id,
        podcast.get("thumbnail").and_then(|v| v.as_str()),
        "thumbnails",
    )
    .await?;

    let news_id = insert_news_row(
        dctx,
        title,
        content,
        category_id,
        url_id,
        thumbnail_url.as_deref(),
    )
    .await?;

    ensure_artist_link(dctx, news_id).await?;
    ensure_metric_row(dctx, news_id).await?;

    if let Some(audio_url) = upload_optional_asset(
        dctx,
        podcast_id,
        podcast.get("audio").and_then(|v| v.as_str()),
        "audio",
    )
    .await?
    {
        insert_podcast_row(dctx, news_id, title, &audio_url).await?;
    }

    let timeline_ids = deploy_timelines_for_news(dctx, timeline_groups, news_id).await?;

    Ok(DeployedPodcast {
        news_id,
        timeline_ids,
    })
}

async fn deploy_timelines_for_news(
    dctx: &DeployContext<'_>,
    timeline_groups: &[Value],
    news_id: i64,
) -> Result<Vec<i64>, CiteError> {
    let mut timeline_ids = Vec::new();

    for group in timeline_groups {
        let Some(entries) = group.get("entries").and_then(|v| v.as_array()) else {
            continue;
        };

        for entry in entries {
            let title = entry
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Untitled");
            let date = entry.get("date").and_then(|v| v.as_str()).unwrap_or("");
            let summary = entry.get("summary").and_then(|v| v.as_str()).unwrap_or("");
            let description = format!("Date: {date}\nSummary: {summary}");
            let url_id = if let Some(url) = entry.get("url").and_then(|v| v.as_str()) {
                Some(ensure_url_id(dctx, Some(url), url, None).await?)
            } else {
                None
            };

            let timeline_payload = build_map(&[
                ("title", Value::String(title.to_string())),
                ("description", Value::String(description)),
                ("created_at", Value::String(chrono_now_rfc3339())),
            ]);
            let timeline_payload = if let Some(url_id) = url_id {
                let mut payload = timeline_payload;
                payload.insert("url_id".into(), Value::Number(url_id.into()));
                payload
            } else {
                timeline_payload
            };

            let timeline_id = insert_row(dctx, "timeline", timeline_payload).await?;

            let link_payload = build_map(&[
                ("timeline_id", Value::Number(timeline_id.into())),
                ("news_id", Value::Number(news_id.into())),
            ]);
            let _ = insert_row(dctx, "timeline_news", link_payload).await?;
            timeline_ids.push(timeline_id);
            eprintln!("  Deployed timeline entry: {}", title.cyan());
        }
    }

    if !timeline_ids.is_empty() {
        eprintln!("  Linked {} timeline entries", timeline_ids.len());
    }
    Ok(timeline_ids)
}

async fn persist_deployment_record(
    ctx: &ProjectContext,
    record: &DeploymentRecord,
) -> Result<(), CiteError> {
    let deployments_dir = ctx.build_dir().join("deployments");
    tokio::fs::create_dir_all(&deployments_dir).await?;
    let path = deployments_dir.join(format!("{}.json", record.deployment_id));
    let json = serde_json::to_string_pretty(record)?;
    tokio::fs::write(path, json).await?;
    Ok(())
}

async fn load_deployment_record(
    ctx: &ProjectContext,
    deployment_id: &str,
) -> Result<(PathBuf, DeploymentRecord), CiteError> {
    let path = ctx
        .build_dir()
        .join("deployments")
        .join(format!("{}.json", deployment_id));
    let json = tokio::fs::read_to_string(&path).await.map_err(|_| {
        CiteError::Deploy(format!(
            "No local deployment record found for '{deployment_id}'. Run deploy first."
        ))
    })?;
    let record: DeploymentRecord = serde_json::from_str(&json)?;
    Ok((path, record))
}

async fn delete_row_by_id(dctx: &DeployContext<'_>, table: &str, id: i64) -> Result<(), CiteError> {
    let url = format!("{}/rest/v1/{table}?id=eq.{id}", dctx.base_url);
    let response = with_auth(dctx.client.delete(&url), dctx.service_key)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CiteError::Deploy(format!(
            "Failed to delete {table} row {id}: HTTP {status} - {body}"
        )));
    }
    Ok(())
}

async fn delete_storage_object(
    dctx: &DeployContext<'_>,
    storage_path: &str,
) -> Result<(), CiteError> {
    let url = format!(
        "{}/storage/v1/object/{STORAGE_BUCKET}/{storage_path}",
        dctx.base_url
    );
    let response = with_auth(dctx.client.delete(&url), dctx.service_key)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CiteError::Deploy(format!(
            "Failed to delete storage object {storage_path}: HTTP {status} - {body}"
        )));
    }
    Ok(())
}

#[instrument(skip(ctx), fields(id = %deployment_id))]
pub async fn rollback(ctx: &ProjectContext, deployment_id: &str) -> Result<(), CiteError> {
    let backend = ctx
        .manifest
        .backend
        .as_ref()
        .ok_or_else(|| CiteError::Deploy("No [backend] section in cite.toml".to_string()))?;

    let service_key = resolve_service_key(backend)?;
    let client = reqwest::Client::new();
    let base_url = backend.staging_url.trim_end_matches('/');
    let (record_path, record) = load_deployment_record(ctx, deployment_id).await?;

    let dctx = DeployContext {
        client: &client,
        base_url,
        service_key: &service_key,
        ctx,
        artist_id: Uuid::nil(), // not used in rollback
    };

    eprintln!(
        "{}",
        format!("Rolling back deployment: {deployment_id}")
            .yellow()
            .bold()
    );

    for timeline_id in &record.timeline_ids {
        delete_row_by_id(&dctx, "timeline", *timeline_id).await?;
        eprintln!("  Cleared timeline {timeline_id}");
    }

    for news_id in &record.news_ids {
        delete_row_by_id(&dctx, "news", *news_id).await?;
        eprintln!("  Cleared news {news_id}");
    }

    if let Err(e) = delete_storage_object(&dctx, &record.storage_path).await {
        eprintln!("  {} {e}", "warning:".yellow().bold());
    } else {
        eprintln!("  Cleared storage object");
    }

    if let Err(e) = tokio::fs::remove_file(&record_path).await {
        eprintln!(
            "  {} Failed to remove local deployment record: {e}",
            "warning:".yellow().bold()
        );
    }

    eprintln!("{}", "Rollback complete".green().bold());
    Ok(())
}

fn resolve_service_key(backend: &crate::manifest::BackendConfig) -> Result<String, CiteError> {
    let key = env::var("CITE_STAGING_SERVICE_KEY")
        .unwrap_or_else(|_| backend.staging_service_key.clone());

    if key.is_empty() {
        Err(CiteError::Deploy(
            "No service key found. Set CITE_STAGING_SERVICE_KEY env var or configure backend.staging_service_key in cite.toml"
                .to_string(),
        ))
    } else {
        Ok(key)
    }
}

fn build_map(fields: &[(&str, Value)]) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for (key, value) in fields {
        map.insert((*key).to_string(), value.clone());
    }
    map
}

fn extract_id(value: &Value) -> Option<i64> {
    match value {
        Value::Array(rows) => rows
            .first()
            .and_then(|row| row.get("id").and_then(|id| id.as_i64())),
        Value::Object(map) => map.get("id").and_then(|id| id.as_i64()),
        _ => None,
    }
}

async fn lookup_row_id(
    dctx: &DeployContext<'_>,
    table: &str,
    field: &str,
    value: &str,
) -> Result<Option<i64>, CiteError> {
    let url = format!(
        "{}/rest/v1/{table}?{field}=eq.{}",
        dctx.base_url,
        encode_url(value)
    );
    let resp = with_auth(dctx.client.get(&url), dctx.service_key)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let rows: Vec<Value> = resp.json().await?;
    Ok(rows
        .first()
        .and_then(|row| row.get("id").and_then(|id| id.as_i64())))
}

async fn insert_row(
    dctx: &DeployContext<'_>,
    table: &str,
    payload: serde_json::Map<String, Value>,
) -> Result<i64, CiteError> {
    let url = format!("{}/rest/v1/{table}", dctx.base_url);
    let response = with_auth(dctx.client.post(&url), dctx.service_key)
        .header("Prefer", "return=representation")
        .json(&payload)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CiteError::Deploy(format!(
            "Failed to insert into {table}: HTTP {status} - {body}"
        )));
    }

    let row: Value = response.json().await?;
    extract_id(&row)
        .ok_or_else(|| CiteError::Deploy(format!("Could not get {table} id from response")))
}

async fn ensure_category_id(
    dctx: &DeployContext<'_>,
    category_name: Option<&str>,
) -> Result<i64, CiteError> {
    let name = category_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("General");
    if let Some(id) = lookup_row_id(dctx, "categories", "name", name).await? {
        return Ok(id);
    }

    insert_row(
        dctx,
        "categories",
        build_map(&[
            ("name", Value::String(name.to_string())),
            (
                "description",
                Value::String("Created automatically by cite-cli".to_string()),
            ),
        ]),
    )
    .await
}

async fn ensure_domain_id(dctx: &DeployContext<'_>, domain_name: &str) -> Result<i64, CiteError> {
    if let Some(id) = lookup_row_id(dctx, "domains", "domain_name", domain_name).await? {
        return Ok(id);
    }

    insert_row(
        dctx,
        "domains",
        build_map(&[
            ("domain_name", Value::String(domain_name.to_string())),
            ("is_trusted", Value::Bool(false)),
        ]),
    )
    .await
}

async fn ensure_url_id(
    dctx: &DeployContext<'_>,
    source_url: Option<&str>,
    fallback_url: &str,
    word_count: Option<i64>,
) -> Result<i64, CiteError> {
    let url_value = source_url
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_url);
    if let Some(id) = lookup_row_id(dctx, "urls", "url", url_value).await? {
        return Ok(id);
    }

    let mut payload = build_map(&[("url", Value::String(url_value.to_string()))]);
    if let Some(count) = word_count {
        payload.insert("word_count".into(), Value::Number(count.into()));
    }
    payload.insert("accessed_at".into(), Value::String(chrono_now_rfc3339()));
    if let Some(source_url) = source_url.filter(|value| !value.trim().is_empty()) {
        if let Some(domain_name) = extract_domain_name(source_url) {
            let domain_id = ensure_domain_id(dctx, &domain_name).await?;
            payload.insert("domain_id".into(), Value::Number(domain_id.into()));
        }
        payload.insert(
            "reliability_score".into(),
            Value::Number(serde_json::Number::from_f64(1.0).unwrap()),
        );
    }

    insert_row(dctx, "urls", payload).await
}

async fn upload_optional_asset(
    dctx: &DeployContext<'_>,
    podcast_id: &str,
    asset: Option<&str>,
    kind: &str,
) -> Result<Option<String>, CiteError> {
    let Some(asset_path) = asset.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };

    let local_path = dctx.ctx.root.join(asset_path);
    if !local_path.exists() {
        return Ok(None);
    }

    let bytes = tokio::fs::read(&local_path).await?;
    let file_name = local_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("asset.bin");
    let storage_path = format!("{}/{}/{}", kind, podcast_id, file_name);
    let mime = local_path
        .extension()
        .and_then(|value| value.to_str())
        .map(mime_for_extension)
        .unwrap_or("application/octet-stream");

    upload_bytes(
        dctx.client,
        dctx.base_url,
        dctx.service_key,
        &storage_path,
        &bytes,
        mime,
    )
    .await
    .map(Some)
}

async fn insert_news_row(
    dctx: &DeployContext<'_>,
    title: &str,
    content: Option<&str>,
    category_id: i64,
    url_id: i64,
    thumbnail: Option<&str>,
) -> Result<i64, CiteError> {
    let mut payload = build_map(&[
        ("title", Value::String(title.to_string())),
        ("category_id", Value::Number(category_id.into())),
        ("url_id", Value::Number(url_id.into())),
        ("published_at", Value::String(chrono_now_rfc3339())),
    ]);

    if let Some(summary) = summarize_content(content) {
        payload.insert("summary".into(), Value::String(summary));
    }
    if let Some(thumbnail) = thumbnail.filter(|value| !value.trim().is_empty()) {
        payload.insert("thumbnail".into(), Value::String(thumbnail.to_string()));
    }

    insert_row(dctx, "news", payload).await
}

async fn ensure_artist_link(dctx: &DeployContext<'_>, news_id: i64) -> Result<(), CiteError> {
    let payload = build_map(&[
        ("artist_id", Value::String(dctx.artist_id.to_string())),
        ("news_id", Value::Number(news_id.into())),
    ]);
    let _ = insert_row(dctx, "artists_news", payload).await?;
    Ok(())
}

async fn ensure_metric_row(dctx: &DeployContext<'_>, news_id: i64) -> Result<(), CiteError> {
    let payload = build_map(&[("news_id", Value::Number(news_id.into()))]);
    let _ = insert_row(dctx, "metric", payload).await?;
    Ok(())
}

async fn insert_podcast_row(
    dctx: &DeployContext<'_>,
    news_id: i64,
    title: &str,
    podcast_url: &str,
) -> Result<(), CiteError> {
    let plan_id = ensure_basic_plan_id(dctx).await?;
    let payload = build_map(&[
        ("news_id", Value::Number(news_id.into())),
        ("subscription_plan_id", Value::Number(plan_id.into())),
        ("title", Value::String(title.to_string())),
        ("podcast_url", Value::String(podcast_url.to_string())),
    ]);
    let _ = insert_row(dctx, "podcasts", payload).await?;
    Ok(())
}

async fn ensure_basic_plan_id(dctx: &DeployContext<'_>) -> Result<i64, CiteError> {
    if let Some(id) = lookup_row_id(dctx, "subscription_plans", "tier_name", "Basic").await? {
        return Ok(id);
    }

    insert_row(
        dctx,
        "subscription_plans",
        build_map(&[
            ("tier_name", Value::String("Basic".to_string())),
            (
                "description",
                Value::String("Default access tier created by cite-cli".to_string()),
            ),
            ("price", Value::from(0.0)),
            ("newsletter_count", Value::Number(0.into())),
            ("max_podcast_duration_minutes", Value::Number(60.into())),
            ("timeline_access", Value::Bool(false)),
        ]),
    )
    .await
}

fn extract_domain_name(source_url: &str) -> Option<String> {
    let without_scheme = source_url
        .split_once("//")
        .map(|(_, rest)| rest)
        .unwrap_or(source_url);
    let domain = without_scheme.split('/').next()?.trim();
    if domain.is_empty() {
        None
    } else {
        Some(domain.to_string())
    }
}

fn summarize_content(content: Option<&str>) -> Option<String> {
    let content = content?.trim();
    if content.is_empty() {
        return None;
    }

    let summary = content
        .split_whitespace()
        .take(60)
        .collect::<Vec<_>>()
        .join(" ");
    if summary.len() < content.len() {
        Some(format!("{summary}..."))
    } else {
        Some(summary)
    }
}

fn word_count(content: &str) -> i64 {
    content.split_whitespace().count() as i64
}

async fn upload_bytes(
    client: &reqwest::Client,
    base_url: &str,
    service_key: &str,
    storage_path: &str,
    bytes: &[u8],
    mime: &str,
) -> Result<String, CiteError> {
    let url = format!("{base_url}/storage/v1/object/{STORAGE_BUCKET}/{storage_path}");

    let response = with_auth(client.post(&url), service_key)
        .header("Content-Type", mime)
        .body(bytes.to_vec())
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CiteError::Deploy(format!(
            "Failed to upload {storage_path}: HTTP {status} - {body}"
        )));
    }

    let public_url = format!("{base_url}/storage/v1/object/public/{STORAGE_BUCKET}/{storage_path}");
    Ok(public_url)
}

fn mime_for_extension(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "md" => "text/markdown",
        "rst" => "text/x-rst",
        "json" => "application/json",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

fn chrono_now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    let (year, month, day) = days_to_date(days as i64);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

fn days_to_date(mut days: i64) -> (i64, i64, i64) {
    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
