use crate::error::CiteError;
use crate::project::ProjectContext;
use colored::Colorize;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::RequestBuilder;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use tracing::instrument;

const STORAGE_BUCKET: &str = "assets";

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

    let deployment_id = uuid::Uuid::new_v4().to_string();

    eprintln!(
        "{}",
        format!("Deploying with deployment_id: {deployment_id}")
            .cyan()
            .bold()
    );

    if dry_run {
        eprintln!("{}", "  DRY RUN - no data will be sent".yellow().bold());
        if let Some(pods) = bundle.get("podcasts").and_then(|v| v.as_array()) {
            eprintln!(
                "{}",
                format!("  Would deploy {} podcast items", pods.len()).cyan()
            );
        }
        if let Some(artist_id) = bundle.get("artist_id").and_then(|v| v.as_str()) {
            eprintln!("{}", format!("  Artist ID: {artist_id}").cyan());
        }
        return Ok(());
    }

    let client = reqwest::Client::new();
    let base_url = backend.staging_url.trim_end_matches('/');

    let project_slug = bundle
        .get("project")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let artist_id = bundle
        .get("artist_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let podcasts: Vec<Value> = bundle
        .get("podcasts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut timeline_map: HashMap<String, Vec<Value>> = HashMap::new();
    if let Some(timelines) = bundle.get("timelines").and_then(|v| v.as_array()) {
        for tl in timelines {
            if let Some(slug) = tl.get("slug").and_then(|v| v.as_str())
                && let Some(pod_id) = slug.strip_suffix("-timeline")
                && let Some(entries) = tl.get("entries").and_then(|v| v.as_array())
            {
                timeline_map.insert(pod_id.to_string(), entries.clone());
            }
        }
    }

    // Upload full content.json to storage as {deployment_id}.json
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

    for pod in &podcasts {
        let title = pod["title"].as_str().unwrap_or("Untitled");
        let pod_id = pod["id"].as_str().unwrap_or("");

        eprintln!("  Processing: {}", title.cyan());

        let category_id = if let Some(cat_name) = pod.get("category").and_then(|v| v.as_str()) {
            resolve_category_id(&client, base_url, &service_key, cat_name).await?
        } else {
            1i64
        };

        let url_id = if let Some(source_url) = pod.get("source_url").and_then(|v| v.as_str()) {
            if !source_url.is_empty() {
                Some(resolve_url_id(&client, base_url, &service_key, source_url).await?)
            } else {
                None
            }
        } else {
            None
        };

        let thumbnail_url = if let Some(thumb) = pod.get("thumbnail").and_then(|v| v.as_str()) {
            let local_path = ctx.root.join(thumb);
            if local_path.exists() {
                let thumb_storage = format!("{project_slug}/thumbnails/{pod_id}/{}", thumb);
                let bytes = tokio::fs::read(&local_path).await?;
                let ext = local_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("bin");
                let mime = mime_for_extension(ext);
                match upload_bytes(
                    &client,
                    base_url,
                    &service_key,
                    &thumb_storage,
                    &bytes,
                    mime,
                )
                .await
                {
                    Ok(url) => Some(url),
                    Err(e) => {
                        eprintln!(
                            "  {} Failed to upload thumbnail: {e}",
                            "warning:".yellow().bold()
                        );
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        // Build news payload
        let mut news_payload = serde_json::Map::new();
        news_payload.insert("title".into(), Value::String(title.to_string()));
        news_payload.insert(
            "summary".into(),
            pod.get("content").cloned().unwrap_or(Value::Null),
        );
        news_payload.insert("category_id".into(), Value::Number(category_id.into()));
        news_payload.insert("published_at".into(), Value::String(chrono_now_rfc3339()));
        news_payload.insert("deployment_id".into(), Value::String(deployment_id.clone()));
        if let Some(url_id) = url_id {
            news_payload.insert("url_id".into(), Value::Number(url_id.into()));
        }
        if let Some(thumb) = &thumbnail_url {
            news_payload.insert("thumbnail".into(), Value::String(thumb.clone()));
        }

        let news_url = format!("{base_url}/rest/v1/news");
        let news_response = with_auth(client.post(&news_url), &service_key)
            .header("Prefer", "return=representation")
            .json(&news_payload)
            .send()
            .await?;

        if !news_response.status().is_success() {
            let status = news_response.status();
            let body = news_response.text().await.unwrap_or_default();
            return Err(CiteError::Deploy(format!(
                "Failed to insert news '{title}': HTTP {status} - {body}"
            )));
        }

        let news_row: Value = news_response.json().await?;
        let news_id = news_row["id"]
            .as_i64()
            .ok_or_else(|| CiteError::Deploy("Could not get news_id from response".to_string()))?;

        // Auto-create artists_news junction
        let junction_payload = serde_json::json!({
            "artist_id": artist_id,
            "news_id": news_id,
            "deployment_id": deployment_id,
        });
        let junction_url = format!("{base_url}/rest/v1/artists_news");
        let j_resp = with_auth(client.post(&junction_url), &service_key)
            .header("Prefer", "resolution=merge-duplicates")
            .json(&junction_payload)
            .send()
            .await?;

        if !j_resp.status().is_success() {
            eprintln!(
                "  {} Failed to link artist to news: HTTP {}",
                "warning:".yellow().bold(),
                j_resp.status()
            );
        }

        // Upload audio if present
        if let Some(audio) = pod.get("audio").and_then(|v| v.as_str()) {
            let local_audio = ctx
                .root
                .join("assets")
                .join(audio.trim_start_matches("assets/"));
            if local_audio.exists() {
                let audio_storage = format!("{project_slug}/audio/{pod_id}/{}", audio);
                let bytes = tokio::fs::read(&local_audio).await?;
                let ext = local_audio
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("bin");
                let mime = mime_for_extension(ext);
                match upload_bytes(
                    &client,
                    base_url,
                    &service_key,
                    &audio_storage,
                    &bytes,
                    mime,
                )
                .await
                {
                    Ok(podcast_url) => {
                        let pod_payload = serde_json::json!({
                            "news_id": news_id,
                            "title": format!("{} Podcast", title),
                            "podcast_url": podcast_url,
                            "deployment_id": deployment_id,
                        });
                        let pod_url = format!("{base_url}/rest/v1/podcasts");
                        let p_resp = with_auth(client.post(&pod_url), &service_key)
                            .header("Prefer", "resolution=merge-duplicates")
                            .json(&pod_payload)
                            .send()
                            .await?;

                        if !p_resp.status().is_success() {
                            let status = p_resp.status();
                            let body = p_resp.text().await.unwrap_or_default();
                            eprintln!(
                                "  {} Failed to insert podcast: HTTP {status} - {body}",
                                "warning:".yellow().bold()
                            );
                        } else {
                            eprintln!("  Uploaded audio to storage");
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "  {} Failed to upload audio: {e}",
                            "warning:".yellow().bold()
                        );
                    }
                }
            }
        }

        // Deploy timeline entries if this podcast has a citation
        if let Some(entries) = timeline_map.get(pod_id) {
            deploy_timeline_for_podcast(
                &client,
                base_url,
                &service_key,
                &deployment_id,
                news_id,
                entries,
            )
            .await?;
        }
    }

    eprintln!(
        "{}",
        format!("Deployment complete (id: {deployment_id})")
            .green()
            .bold()
    );

    Ok(())
}

async fn deploy_timeline_for_podcast(
    client: &reqwest::Client,
    base_url: &str,
    service_key: &str,
    deployment_id: &str,
    news_id: i64,
    entries: &[Value],
) -> Result<(), CiteError> {
    for entry in entries {
        let tl_title = entry
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled");
        let tl_date = entry.get("date").and_then(|v| v.as_str()).unwrap_or("");
        let tl_summary = entry.get("summary").and_then(|v| v.as_str()).unwrap_or("");
        let description = format!("Date: {tl_date}\nSummary: {tl_summary}");

        let tl_payload = serde_json::json!({
            "title": tl_title,
            "description": description,
            "deployment_id": deployment_id,
        });
        let tl_url = format!("{base_url}/rest/v1/timeline");
        match with_auth(client.post(&tl_url), service_key)
            .header("Prefer", "return=representation")
            .json(&tl_payload)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(tl_row) = resp.json::<Value>().await
                    && let Some(tl_id) = tl_row.get("id").and_then(|v| v.as_i64())
                {
                    let tn_payload = serde_json::json!({
                        "timeline_id": tl_id,
                        "news_id": news_id,
                        "deployment_id": deployment_id,
                    });
                    let tn_url = format!("{base_url}/rest/v1/timeline_news");
                    let _ = with_auth(client.post(&tn_url), service_key)
                        .header("Prefer", "resolution=merge-duplicates")
                        .json(&tn_payload)
                        .send()
                        .await;
                }
                eprintln!("  Deployed timeline entry: {tl_title}");
            }
            Err(e) => {
                eprintln!(
                    "  {} Failed to deploy timeline: {e}",
                    "warning:".yellow().bold()
                );
            }
            _ => {}
        }
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

    eprintln!(
        "{}",
        format!("Rolling back deployment: {deployment_id}")
            .yellow()
            .bold()
    );

    let tables = ["podcasts", "artists_news", "news"];
    for table in &tables {
        let url = format!(
            "{base_url}/rest/v1/{table}?deployment_id=eq.{}",
            encode_url(deployment_id)
        );
        let response = with_auth(client.delete(&url), &service_key).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            eprintln!(
                "{}",
                format!("  Failed to rollback {table}: HTTP {status}")
                    .red()
                    .bold()
            );
            eprintln!("    {body}");
        } else {
            eprintln!("{}", format!("  Cleared {table}").green());
        }
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

async fn resolve_category_id(
    client: &reqwest::Client,
    base_url: &str,
    service_key: &str,
    category_name: &str,
) -> Result<i64, CiteError> {
    let url = format!(
        "{base_url}/rest/v1/categories?name=eq.{}",
        encode_url(category_name)
    );
    let resp = with_auth(client.get(&url), service_key).send().await?;

    if resp.status().is_success() {
        let rows: Vec<Value> = resp.json().await?;
        if let Some(row) = rows.first()
            && let Some(id) = row.get("id").and_then(|v| v.as_i64())
        {
            return Ok(id);
        }
    }

    Err(CiteError::Deploy(format!(
        "Category '{category_name}' not found in database. Available categories can be viewed in Supabase."
    )))
}

async fn resolve_url_id(
    client: &reqwest::Client,
    base_url: &str,
    service_key: &str,
    source_url: &str,
) -> Result<i64, CiteError> {
    let encoded = encode_url(source_url);
    let url = format!("{base_url}/rest/v1/urls?url=eq.{encoded}");
    let resp = with_auth(client.get(&url), service_key).send().await?;

    if resp.status().is_success() {
        let rows: Vec<Value> = resp.json().await?;
        if let Some(row) = rows.first()
            && let Some(id) = row.get("id").and_then(|v| v.as_i64())
        {
            return Ok(id);
        }
    }

    // Create new url record
    let payload = serde_json::json!({ "url": source_url });
    let insert_url = format!("{base_url}/rest/v1/urls");
    let insert_resp = with_auth(client.post(&insert_url), service_key)
        .header("Prefer", "return=representation")
        .json(&payload)
        .send()
        .await?;

    if !insert_resp.status().is_success() {
        let status = insert_resp.status();
        let body = insert_resp.text().await.unwrap_or_default();
        return Err(CiteError::Deploy(format!(
            "Failed to create url record: HTTP {status} - {body}"
        )));
    }

    let row: Value = insert_resp.json().await?;
    row.get("id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| CiteError::Deploy("Could not get url_id from response".to_string()))
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

    let response = with_auth(client.post(&url), &service_key)
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
