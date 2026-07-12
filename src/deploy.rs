use crate::error::CiteError;
use crate::manifest::BackendConfig;
use crate::project::ProjectContext;
use colored::Colorize;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use tracing::instrument;
use uuid::Uuid;

const STORAGE_BUCKET: &str = "assets";

#[derive(Debug, Clone)]
struct DeployContext {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    bearer: String,
    root: PathBuf,
    project_name: String,
    artist_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeploymentRecord {
    deployment_id: String,
    storage_path: String,
    news_ids: Vec<i64>,
    timeline_ids: Vec<i64>,
    #[serde(default)]
    asset_paths: Vec<String>,
}

#[derive(Debug)]
struct DeployedPodcast {
    news_id: i64,
    timeline_ids: Vec<i64>,
    asset_paths: Vec<String>,
}

#[derive(Debug)]
struct UploadedAsset {
    storage_path: String,
    public_url: String,
}

fn with_auth(builder: RequestBuilder, api_key: &str, bearer: &str) -> RequestBuilder {
    builder
        .header("apikey", api_key)
        .header("Authorization", format!("Bearer {bearer}"))
}

fn encode_url(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn build_context(
    ctx: &ProjectContext,
    backend: &BackendConfig,
    project_name: String,
    artist_id: Uuid,
) -> Result<DeployContext, CiteError> {
    let session = load_session();
    Ok(DeployContext {
        client: reqwest::Client::new(),
        base_url: backend.staging_url.trim_end_matches('/').to_string(),
        api_key: backend.staging_service_key.clone(),
        bearer: resolve_bearer(backend, session.as_ref())?,
        root: ctx.root.clone(),
        project_name,
        artist_id,
    })
}

async fn ensure_success(
    response: reqwest::Response,
    context: impl std::fmt::Display,
) -> Result<reqwest::Response, CiteError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(CiteError::Deploy(format!(
        "{context}: HTTP {status} - {body}"
    )))
}

#[instrument(skip(ctx), fields(project = %ctx.manifest.project.name, dry_run))]
pub async fn deploy(ctx: &ProjectContext, dry_run: bool) -> Result<(), CiteError> {
    let backend = ctx
        .manifest
        .backend
        .as_ref()
        .ok_or_else(|| CiteError::Deploy("No [backend] section in cite.toml".to_string()))?;

    let bundle_path = ctx.build_dir().join("content.json");
    if !bundle_path.exists() {
        return Err(CiteError::Deploy(
            "No build artifact found. Run 'cite-cli build' first.".to_string(),
        ));
    }
    let bundle_str = std::fs::read_to_string(&bundle_path)?;
    let bundle: Value = serde_json::from_str(&bundle_str)?;
    let deployment_id = Uuid::new_v4().to_string();
    let project_name = bundle
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
    let dctx = build_context(ctx, backend, project_name.clone(), artist_id)?;
    let storage_path = format!("{artist_id}/{project_name}/{deployment_id}.json");
    let bundle_bytes = serde_json::to_vec_pretty(&bundle)?;

    let mut record = DeploymentRecord {
        deployment_id: deployment_id.clone(),
        storage_path: storage_path.clone(),
        news_ids: Vec::new(),
        timeline_ids: Vec::new(),
        asset_paths: Vec::new(),
    };

    ensure_artist_exists(&dctx, artist_id).await?;
    let plan_id = fetch_subscription_plan(&dctx).await?;

    for pod in &podcasts {
        let deployed = deploy_podcast(&dctx, pod, &timelines, artist_id, plan_id).await?;
        record.news_ids.push(deployed.news_id);
        record.timeline_ids.extend(deployed.timeline_ids);
        record.asset_paths.extend(deployed.asset_paths);
    }

    let public_bundle_url =
        upload_bytes(&dctx, &storage_path, &bundle_bytes, "application/json").await?;
    eprintln!("  Uploaded bundle to {}", public_bundle_url.cyan());

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
    dctx: &DeployContext,
    podcast: &Value,
    timeline_groups: &[Value],
    artist_id: Uuid,
    plan_id: i64,
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

    let mut asset_paths = Vec::new();

    let news_id = insert_news_row(dctx, title, content, category_id, url_id, None).await?;
    ensure_artist_link(dctx, artist_id, news_id).await?;
    ensure_metric_row(dctx, news_id).await?;

    let timeline_ids = deploy_timelines_for_news(dctx, timeline_groups, news_id).await?;

    if let Some(asset) = upload_optional_asset(
        dctx,
        podcast.get("thumbnail").and_then(|v| v.as_str()),
        "image",
    )
    .await?
    {
        update_news_thumbnail(dctx, news_id, &asset.public_url).await?;
        asset_paths.push(asset.storage_path);
    }

    if let Some(audio) =
        upload_optional_asset(dctx, podcast.get("audio").and_then(|v| v.as_str()), "audio").await?
    {
        insert_podcast_row(dctx, news_id, title, &audio.public_url, plan_id).await?;
        asset_paths.push(audio.storage_path);
    }

    Ok(DeployedPodcast {
        news_id,
        timeline_ids,
        asset_paths,
    })
}

async fn deploy_timelines_for_news(
    dctx: &DeployContext,
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

            let mut timeline_payload = build_map(&[
                ("title", Value::String(title.to_string())),
                ("description", Value::String(description)),
                ("created_at", Value::String(now_rfc3339())),
            ]);
            if let Some(url_id) = url_id {
                timeline_payload.insert("url_id".into(), Value::Number(url_id.into()));
            }

            let timeline_id = insert_row(dctx, "timeline", timeline_payload).await?;

            insert_row(
                dctx,
                "timeline_news",
                build_map(&[
                    ("timeline_id", Value::Number(timeline_id.into())),
                    ("news_id", Value::Number(news_id.into())),
                ]),
            )
            .await?;
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
) -> Result<(std::path::PathBuf, DeploymentRecord), CiteError> {
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

async fn delete_row_by_id(dctx: &DeployContext, table: &str, id: i64) -> Result<(), CiteError> {
    let url = format!("{}/rest/v1/{table}?id=eq.{id}", dctx.base_url);
    let response = with_auth(dctx.client.delete(&url), &dctx.api_key, &dctx.bearer)
        .send()
        .await?;
    ensure_success(response, format!("Failed to delete {table} row {id}")).await?;
    Ok(())
}

async fn delete_storage_object(dctx: &DeployContext, storage_path: &str) -> Result<(), CiteError> {
    let url = format!(
        "{}/storage/v1/object/{STORAGE_BUCKET}/{storage_path}",
        dctx.base_url
    );
    let response = with_auth(dctx.client.delete(&url), &dctx.api_key, &dctx.bearer)
        .send()
        .await?;
    ensure_success(
        response,
        format!("Failed to delete storage object {storage_path}"),
    )
    .await?;
    Ok(())
}

#[instrument(skip(ctx), fields(id = %deployment_id))]
pub async fn rollback(ctx: &ProjectContext, deployment_id: &str) -> Result<(), CiteError> {
    let backend = ctx
        .manifest
        .backend
        .as_ref()
        .ok_or_else(|| CiteError::Deploy("No [backend] section in cite.toml".to_string()))?;

    let (record_path, record) = load_deployment_record(ctx, deployment_id).await?;
    let artist_id = Uuid::parse_str(&ctx.manifest.project.artist_id).map_err(|_| {
        CiteError::Deploy("artist_id in cite.toml must be a valid UUID".to_string())
    })?;
    let dctx = build_context(ctx, backend, ctx.manifest.project.name.clone(), artist_id)?;

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

    for asset_path in &record.asset_paths {
        if let Err(e) = delete_storage_object(&dctx, asset_path).await {
            eprintln!("  {} {e}", "warning:".yellow().bold());
        } else {
            eprintln!("  Cleared asset {asset_path}");
        }
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

#[derive(Serialize, Deserialize, Debug)]
struct Session {
    access_token: String,
    refresh_token: String,
    email: String,
}

fn session_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".cite").join("session.json")
}

fn load_session() -> Option<Session> {
    let path = session_path();
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_session(session: &Session) -> Result<(), CiteError> {
    let path = session_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(session)?)?;
    Ok(())
}

fn resolve_bearer(backend: &BackendConfig, session: Option<&Session>) -> Result<String, CiteError> {
    if let Some(s) = session {
        return Ok(s.access_token.clone());
    }
    if backend.staging_service_key.is_empty() {
        Err(CiteError::Deploy(
            "Not logged in and no backend.staging_service_key configured. Run 'cite-cli login' or set the key in cite.toml"
                .to_string(),
        ))
    } else {
        Ok(backend.staging_service_key.clone())
    }
}

pub async fn login(
    ctx: &ProjectContext,
    email: Option<String>,
    password: Option<String>,
) -> Result<(), CiteError> {
    let backend = ctx
        .manifest
        .backend
        .as_ref()
        .ok_or_else(|| CiteError::Deploy("No [backend] section in cite.toml".to_string()))?;
    if backend.staging_service_key.is_empty() {
        return Err(CiteError::Deploy(
            "backend.staging_service_key (Supabase anon/publishable key) is required for login"
                .to_string(),
        ));
    }

    let email = match email {
        Some(e) => e,
        None => {
            eprint!("Email: ");
            let mut s = String::new();
            std::io::stdin().read_line(&mut s)?;
            s.trim().to_string()
        }
    };
    let password = match password {
        Some(p) => p,
        None => {
            eprint!("Password: ");
            let mut s = String::new();
            std::io::stdin().read_line(&mut s)?;
            s.trim().to_string()
        }
    };

    let client = reqwest::Client::new();
    let url = format!(
        "{}/auth/v1/token?grant_type=password",
        backend.staging_url.trim_end_matches('/')
    );
    let response = client
        .post(&url)
        .header("apikey", &backend.staging_service_key)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "email": email, "password": password }))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(CiteError::Deploy(format!(
            "Login failed: HTTP {status} - {body}"
        )));
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
        refresh_token: String,
    }
    let token: TokenResponse = serde_json::from_str(&body)
        .map_err(|e| CiteError::Deploy(format!("Invalid login response: {e}")))?;

    let session = Session {
        access_token: token.access_token.clone(),
        refresh_token: token.refresh_token,
        email: email.clone(),
    };
    save_session(&session)?;
    eprintln!("{}", format!("Logged in as {email}").green().bold());

    match fetch_user_artists(backend, &token.access_token).await {
        Ok(artists) if artists.is_empty() => {
            eprintln!("{}", "No artist linked to this account.".yellow().bold());
            match prompt_create_artist(backend, &token.access_token).await? {
                Some((id, name)) => {
                    eprintln!(
                        "{}",
                        format!("Created artist '{name}' ({id})").green().bold()
                    );
                }
                None => {
                    eprintln!("  Skipped artist creation");
                }
            }
        }
        Ok(artists) => {
            eprintln!("  Associated artist(s):");
            for (id, name) in &artists {
                eprintln!("    - {name} ({id})");
            }
        }
        Err(e) => {
            eprintln!(
                "  {} Could not fetch artists: {e}",
                "warning:".yellow().bold()
            );
        }
    }

    Ok(())
}

async fn fetch_user_artists(
    backend: &BackendConfig,
    token: &str,
) -> Result<Vec<(String, String)>, CiteError> {
    let url = format!(
        "{}/rest/v1/artists?select=id,name",
        backend.staging_url.trim_end_matches('/')
    );
    let resp = with_auth(
        reqwest::Client::new().get(&url),
        &backend.staging_service_key,
        token,
    )
    .send()
    .await?;
    if !resp.status().is_success() {
        return Err(CiteError::Deploy(format!(
            "Failed to fetch artists: HTTP {}",
            resp.status()
        )));
    }
    let rows: Vec<Value> = resp.json().await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let id = r.get("id")?.as_str()?.to_string();
            let name = r.get("name")?.as_str()?.to_string();
            Some((id, name))
        })
        .collect())
}

fn prompt_line(label: &str) -> Result<String, CiteError> {
    eprint!("{label}");
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

async fn prompt_create_artist(
    backend: &BackendConfig,
    token: &str,
) -> Result<Option<(String, String)>, CiteError> {
    let name = prompt_line("Artist name: ")?;
    if name.is_empty() {
        return Ok(None);
    }
    let description = prompt_line("Description (optional): ")?;
    let website = prompt_line("Website URL (optional): ")?;

    let mut payload = build_map(&[("name", Value::String(name.clone()))]);
    if !description.is_empty() {
        payload.insert("description".into(), Value::String(description));
    }
    if !website.is_empty() {
        payload.insert("website_url".into(), Value::String(website));
    }

    let url = format!(
        "{}/rest/v1/artists",
        backend.staging_url.trim_end_matches('/')
    );
    let resp = with_auth(
        reqwest::Client::new().post(&url),
        &backend.staging_service_key,
        token,
    )
    .header("Prefer", "return=representation")
    .json(&payload)
    .send()
    .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(CiteError::Deploy(format!(
            "Failed to create artist: HTTP {status} - {body}"
        )));
    }
    let row: Value = resp.json().await?;
    let id = row
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(Some((id, name)))
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
    dctx: &DeployContext,
    table: &str,
    field: &str,
    value: &str,
) -> Result<Option<i64>, CiteError> {
    let url = format!(
        "{}/rest/v1/{table}?{field}=eq.{}",
        &dctx.base_url,
        encode_url(value)
    );
    let resp = with_auth(dctx.client.get(&url), &dctx.api_key, &dctx.bearer)
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
    dctx: &DeployContext,
    table: &str,
    payload: serde_json::Map<String, Value>,
) -> Result<i64, CiteError> {
    let url = format!("{}/rest/v1/{table}", dctx.base_url);
    let response = with_auth(dctx.client.post(&url), &dctx.api_key, &dctx.bearer)
        .header("Prefer", "return=representation")
        .json(&payload)
        .send()
        .await?;
    let response = ensure_success(response, format!("Failed to insert into {table}")).await?;

    let row: Value = response.json().await?;
    extract_id(&row)
        .ok_or_else(|| CiteError::Deploy(format!("Could not get {table} id from response")))
}

async fn ensure_artist_exists(dctx: &DeployContext, artist_id: Uuid) -> Result<(), CiteError> {
    let url = format!("{}/rest/v1/artists?id=eq.{}", &dctx.base_url, artist_id);
    let resp = with_auth(dctx.client.get(&url), &dctx.api_key, &dctx.bearer)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(CiteError::Deploy(format!(
            "Failed to verify artist {artist_id}: HTTP {}",
            resp.status()
        )));
    }
    let rows: Vec<Value> = resp.json().await?;
    if rows.is_empty() {
        return Err(CiteError::Deploy(format!(
            "Artist '{artist_id}' does not exist in the database. Create the artist before deploying."
        )));
    }
    Ok(())
}

async fn ensure_category_id(
    dctx: &DeployContext,
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

async fn ensure_domain_id(dctx: &DeployContext, domain_name: &str) -> Result<i64, CiteError> {
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
    dctx: &DeployContext,
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
    payload.insert("accessed_at".into(), Value::String(now_rfc3339()));
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
    dctx: &DeployContext,
    asset: Option<&str>,
    kind: &str,
) -> Result<Option<UploadedAsset>, CiteError> {
    let Some(asset_path) = asset.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };

    let local_path = dctx.root.join(asset_path);
    if !local_path.exists() {
        return Ok(None);
    }

    let bytes = tokio::fs::read(&local_path).await?;
    let file_name = local_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("asset.bin");
    let storage_path = format!(
        "{}/{}/{}/{}",
        dctx.artist_id, dctx.project_name, kind, file_name
    );
    let mime = local_path
        .extension()
        .and_then(|value| value.to_str())
        .map(mime_for_extension)
        .unwrap_or("application/octet-stream");

    let public_url = upload_bytes(dctx, &storage_path, &bytes, mime).await?;

    Ok(Some(UploadedAsset {
        storage_path,
        public_url,
    }))
}

async fn insert_news_row(
    dctx: &DeployContext,
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
        ("published_at", Value::String(now_rfc3339())),
    ]);

    if let Some(summary) = summarize_content(content) {
        payload.insert("summary".into(), Value::String(summary));
    }
    if let Some(thumbnail) = thumbnail.filter(|value| !value.trim().is_empty()) {
        payload.insert("thumbnail".into(), Value::String(thumbnail.to_string()));
    }

    insert_row(dctx, "news", payload).await
}

async fn ensure_artist_link(
    dctx: &DeployContext,
    artist_id: Uuid,
    news_id: i64,
) -> Result<(), CiteError> {
    insert_row(
        dctx,
        "artists_news",
        build_map(&[
            ("artist_id", Value::String(artist_id.to_string())),
            ("news_id", Value::Number(news_id.into())),
        ]),
    )
    .await?;
    Ok(())
}

async fn update_row(
    dctx: &DeployContext,
    table: &str,
    id: i64,
    payload: serde_json::Map<String, Value>,
) -> Result<(), CiteError> {
    let url = format!("{}/rest/v1/{table}?id=eq.{id}", dctx.base_url);
    let response = with_auth(dctx.client.patch(&url), &dctx.api_key, &dctx.bearer)
        .header("Prefer", "return=representation")
        .json(&payload)
        .send()
        .await?;
    ensure_success(response, format!("Failed to update {table} {id}")).await?;
    Ok(())
}

async fn update_news_thumbnail(
    dctx: &DeployContext,
    news_id: i64,
    thumbnail: &str,
) -> Result<(), CiteError> {
    update_row(
        dctx,
        "news",
        news_id,
        build_map(&[("thumbnail", Value::String(thumbnail.to_string()))]),
    )
    .await
}

async fn ensure_metric_row(dctx: &DeployContext, news_id: i64) -> Result<(), CiteError> {
    insert_row(
        dctx,
        "metric",
        build_map(&[("news_id", Value::Number(news_id.into()))]),
    )
    .await?;
    Ok(())
}

async fn insert_podcast_row(
    dctx: &DeployContext,
    news_id: i64,
    title: &str,
    podcast_url: &str,
    plan_id: i64,
) -> Result<(), CiteError> {
    insert_row(
        dctx,
        "podcasts",
        build_map(&[
            ("news_id", Value::Number(news_id.into())),
            ("subscription_plan_id", Value::Number(plan_id.into())),
            ("title", Value::String(title.to_string())),
            ("podcast_url", Value::String(podcast_url.to_string())),
        ]),
    )
    .await?;
    Ok(())
}

async fn fetch_subscription_plan(dctx: &DeployContext) -> Result<i64, CiteError> {
    let url = format!(
        "{}/rest/v1/user_subscriptions?select=subscription_plan_id&limit=1",
        dctx.base_url
    );
    let resp = with_auth(dctx.client.get(&url), &dctx.api_key, &dctx.bearer)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(CiteError::Deploy(format!(
            "Failed to fetch subscription: HTTP {}",
            resp.status()
        )));
    }
    let rows: Vec<Value> = resp.json().await?;
    rows.first()
        .and_then(|r| r.get("subscription_plan_id").and_then(|v| v.as_i64()))
        .ok_or_else(|| {
            CiteError::Deploy(
                "No active subscription found for this account. Subscribe before deploying."
                    .to_string(),
            )
        })
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
    dctx: &DeployContext,
    storage_path: &str,
    bytes: &[u8],
    mime: &str,
) -> Result<String, CiteError> {
    let base_url = &dctx.base_url;
    let url = format!("{base_url}/storage/v1/object/{STORAGE_BUCKET}/{storage_path}");

    let response = with_auth(dctx.client.post(&url), &dctx.api_key, &dctx.bearer)
        .header("Content-Type", mime)
        .body(bytes.to_vec())
        .send()
        .await?;

    ensure_success(response, format!("Failed to upload {storage_path}")).await?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::BackendConfig;

    fn backend(url: &str, key: &str) -> BackendConfig {
        BackendConfig {
            staging_url: url.to_string(),
            staging_service_key: key.to_string(),
        }
    }

    #[test]
    fn test_resolve_bearer_uses_inline_when_no_session() {
        assert_eq!(
            resolve_bearer(&backend("https://x.supabase.co", "inline"), None).unwrap(),
            "inline"
        );
    }

    #[test]
    fn test_resolve_bearer_prefers_session() {
        let session = Session {
            access_token: "user-jwt".into(),
            refresh_token: "refresh".into(),
            email: "a@b.c".into(),
        };
        assert_eq!(
            resolve_bearer(&backend("https://x.supabase.co", "inline"), Some(&session)).unwrap(),
            "user-jwt"
        );
    }

    #[test]
    fn test_resolve_bearer_errors_when_empty() {
        assert!(resolve_bearer(&backend("https://x.supabase.co", ""), None).is_err());
    }

    #[test]
    fn test_extract_domain_name() {
        assert_eq!(
            extract_domain_name("https://example.com/path"),
            Some("example.com".to_string())
        );
        assert_eq!(
            extract_domain_name("http://a.b.c/d"),
            Some("a.b.c".to_string())
        );
        assert_eq!(
            extract_domain_name("scheme-less.example.com/foo"),
            Some("scheme-less.example.com".to_string())
        );
        assert_eq!(extract_domain_name(""), None);
    }

    #[test]
    fn test_summarize_content() {
        let long = "word ".repeat(80);
        let s = summarize_content(Some(&long)).unwrap();
        assert!(s.ends_with("..."));
        assert!(s.len() < long.len());
        assert_eq!(summarize_content(Some("   ")), None);
        assert_eq!(summarize_content(None), None);
    }

    #[test]
    fn test_word_count() {
        assert_eq!(word_count("one two three"), 3);
        assert_eq!(word_count(""), 0);
    }

    #[test]
    fn test_build_map() {
        let m = build_map(&[("a", Value::String("x".into()))]);
        assert_eq!(m.get("a").unwrap(), "x");
    }

    #[test]
    fn test_extract_id_from_array() {
        let v = serde_json::json!([{ "id": 42 }]);
        assert_eq!(extract_id(&v), Some(42));
    }

    #[test]
    fn test_now_rfc3339_format() {
        assert!(now_rfc3339().contains('T'));
    }

    #[test]
    fn test_encode_url_escapes_special_chars() {
        assert!(encode_url("a b/c").contains('%'));
    }
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use crate::compiler;
    use crate::manifest::{
        BackendConfig, BuildConfig, CompilerConfig, Manifest, ProjectConfig, ValidationConfig,
    };
    use crate::project::ProjectContext;
    use httpmock::Method::PATCH;
    use httpmock::prelude::*;
    use std::path::Path;

    fn write_project(dir: &Path, staging_url: &str) {
        let manifest = Manifest {
            project: ProjectConfig {
                name: "test".into(),
                language: "en".into(),
                metadata_file: "metadata.yml".into(),
                artist_id: "11111111-1111-1111-1111-111111111111".into(),
            },
            build: BuildConfig::default(),
            backend: Some(BackendConfig {
                staging_url: staging_url.into(),
                staging_service_key: "key".into(),
            }),
            compiler: CompilerConfig::default(),
            assets: crate::manifest::AssetsConfig::default(),
            validation: ValidationConfig::default(),
        };
        std::fs::write(dir.join("cite.toml"), toml::to_string(&manifest).unwrap()).unwrap();
        std::fs::write(
            dir.join("metadata.yml"),
            r#"
podcasts:
  - id: "p1"
    title: "Episode One"
    file: content/episode.md
    source_url: "https://example.com/episode"
    category: tech
    thumbnail: assets/image/cover.png
    audio: assets/audio/episode.mp3
    citation: content/papers.bib
"#,
        )
        .unwrap();

        std::fs::create_dir_all(dir.join("content")).unwrap();
        std::fs::write(
            dir.join("content/episode.md"),
            "# Episode One\nWelcome to the show.",
        )
        .unwrap();
        std::fs::write(
            dir.join("content/papers.bib"),
            r#"
@article{ref2023,
  title = {A Great Paper},
  author = {Doe, J.},
  year = {2023},
  month = mar,
  abstract = {Important findings.},
}
"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("assets/image")).unwrap();
        std::fs::write(dir.join("assets/image/cover.png"), "png").unwrap();
        std::fs::create_dir_all(dir.join("assets/audio")).unwrap();
        std::fs::write(dir.join("assets/audio/episode.mp3"), "mp3").unwrap();
    }

    async fn setup(staging_url: &str) -> (tempfile::TempDir, ProjectContext) {
        let dir = tempfile::tempdir().unwrap();
        write_project(dir.path(), staging_url);
        let ctx = ProjectContext::load(dir.path()).unwrap();
        compiler::compile(&ctx, true).await.unwrap();
        (dir, ctx)
    }

    #[tokio::test]
    async fn test_deploy_inserts_and_rollback_deletes() {
        let server = MockServer::start();
        let base = server.base_url();

        let news = server.mock(|w, t| {
            w.method(POST).path("/rest/v1/news");
            t.status(200).json_body(serde_json::json!([{ "id": 1 }]));
        });
        let news_patch = server.mock(|w, t| {
            w.method(PATCH).path("/rest/v1/news");
            t.status(200).json_body(serde_json::json!([{ "id": 1 }]));
        });
        let artists_news = server.mock(|w, t| {
            w.method(POST).path("/rest/v1/artists_news");
            t.status(200).json_body(serde_json::json!([{ "id": 1 }]));
        });
        let metric = server.mock(|w, t| {
            w.method(POST).path("/rest/v1/metric");
            t.status(200).json_body(serde_json::json!([{ "id": 1 }]));
        });
        let categories = server.mock(|w, t| {
            w.method(POST).path("/rest/v1/categories");
            t.status(200).json_body(serde_json::json!([{ "id": 1 }]));
        });
        let urls = server.mock(|w, t| {
            w.method(POST).path("/rest/v1/urls");
            t.status(200).json_body(serde_json::json!([{ "id": 1 }]));
        });
        let domains = server.mock(|w, t| {
            w.method(POST).path("/rest/v1/domains");
            t.status(200).json_body(serde_json::json!([{ "id": 1 }]));
        });
        let plans_get = server.mock(|w, t| {
            w.method(GET).path("/rest/v1/user_subscriptions");
            t.status(200)
                .json_body(serde_json::json!([{ "subscription_plan_id": 3 }]));
        });
        let podcasts = server.mock(|w, t| {
            w.method(POST).path("/rest/v1/podcasts");
            t.status(200).json_body(serde_json::json!([{ "id": 1 }]));
        });
        let timeline = server.mock(|w, t| {
            w.method(POST).path("/rest/v1/timeline");
            t.status(200).json_body(serde_json::json!([{ "id": 1 }]));
        });
        let timeline_news = server.mock(|w, t| {
            w.method(POST).path("/rest/v1/timeline_news");
            t.status(200).json_body(serde_json::json!([{ "id": 1 }]));
        });
        let storage_post = server.mock(|w, t| {
            w.method(POST).path_contains("/storage/v1/object");
            t.status(200);
        });
        let artists_get = server.mock(|w, t| {
            w.method(GET).path_contains("/rest/v1/artists");
            t.status(200)
                .json_body(serde_json::json!([{ "id": "11111111-1111-1111-1111-111111111111" }]));
        });
        let _get_fallback = server.mock(|w, t| {
            w.method(GET).path_contains("/rest/v1/");
            t.status(200).json_body(serde_json::json!([]));
        });
        let del_timeline = server.mock(|w, t| {
            w.method(DELETE).path("/rest/v1/timeline");
            t.status(200).json_body(serde_json::json!([]));
        });
        let del_news = server.mock(|w, t| {
            w.method(DELETE).path("/rest/v1/news");
            t.status(200).json_body(serde_json::json!([]));
        });
        let storage_del = server.mock(|w, t| {
            w.method(DELETE).path_contains("/storage/v1/object");
            t.status(200);
        });
        let _del_fallback = server.mock(|w, t| {
            w.method(DELETE).path_contains("/rest/v1/");
            t.status(200).json_body(serde_json::json!([]));
        });

        let (_dir, ctx) = setup(&base).await;
        deploy(&ctx, false).await.expect("deploy should succeed");

        assert_eq!(news.hits(), 1, "one news row");
        assert_eq!(news_patch.hits(), 1, "thumbnail patched after upload");
        assert_eq!(artists_news.hits(), 1, "artist link");
        assert_eq!(metric.hits(), 1, "metric row");
        assert_eq!(categories.hits(), 1, "category created");
        assert!(urls.hits() >= 1, "url created");
        assert_eq!(domains.hits(), 1, "domain from source url");
        assert!(
            plans_get.hits() >= 1,
            "subscription plan fetched from user_subscriptions"
        );
        assert_eq!(podcasts.hits(), 1, "podcast row");
        assert_eq!(timeline.hits(), 1, "timeline from citation");
        assert_eq!(timeline_news.hits(), 1, "timeline link");
        assert!(storage_post.hits() >= 1, "bundle/asset uploads");
        assert!(artists_get.hits() >= 1, "artist existence checked");
        assert_eq!(
            std::fs::read_dir(ctx.build_dir().join("deployments"))
                .unwrap()
                .count(),
            1,
            "deployment record persisted"
        );

        let deployments_dir = ctx.build_dir().join("deployments");
        let id = std::fs::read_dir(&deployments_dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .to_string_lossy()
            .trim_end_matches(".json")
            .to_string();

        rollback(&ctx, &id).await.expect("rollback should succeed");

        assert_eq!(del_timeline.hits(), 1, "timeline deleted");
        assert_eq!(del_news.hits(), 1, "news deleted");
        assert!(storage_del.hits() >= 1, "storage object deleted");
        assert_eq!(
            std::fs::read_dir(&deployments_dir).unwrap().count(),
            0,
            "local deployment record removed"
        );
    }
}
