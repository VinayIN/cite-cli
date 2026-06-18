use crate::error::CiteError;
use crate::project::ProjectContext;
use colored::Colorize;
use serde_json::Value;
use std::env;
use std::path::Path;
use tracing::instrument;

const TABLES: &[&str] = &["artists", "news", "podcasts"];
const STORAGE_BUCKET: &str = "assets";

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
    let mut bundle: Value = serde_json::from_str(&bundle_str)?;

    let deployment_id = uuid::Uuid::new_v4().to_string();

    eprintln!(
        "{}",
        format!("Deploying with deployment_id: {deployment_id}")
            .cyan()
            .bold()
    );

    if dry_run {
        eprintln!("{}", "  DRY RUN - no data will be sent".yellow().bold());
        for table in TABLES {
            if let Some(items) = bundle.get(table).and_then(|v| v.as_array())
                && !items.is_empty()
            {
                eprintln!("{}", format!("  Would upsert {} {} items", items.len(), table).cyan());
            }
        }
        if has_assets(&bundle) {
            eprintln!("{}", "  Would upload assets to storage".cyan());
        }
        return Ok(());
    }

    let client = reqwest::Client::new();
    let base_url = backend.staging_url.trim_end_matches('/');

    // Upload assets to storage and rewrite URLs
    let build_dir = ctx.build_dir();
    upload_assets(&build_dir, &client, base_url, &service_key, &mut bundle).await?;

    // Upsert data to tables
    for table in TABLES {
        if let Some(items) = bundle.get_mut(table).and_then(|v| v.as_array_mut()) {
            if items.is_empty() {
                continue;
            }

            for item in items.iter_mut() {
                if let Some(obj) = item.as_object_mut() {
                    obj.insert(
                        "deployment_id".to_string(),
                        Value::String(deployment_id.clone()),
                    );
                }
            }

            eprintln!(
                "  {}: {} items",
                table.cyan(),
                items.len()
            );

            let url = format!("{base_url}/rest/v1/{table}");
            let response = client
                .post(&url)
                .header("apikey", &service_key)
                .header("Authorization", format!("Bearer {service_key}"))
                .header("Content-Type", "application/json")
                .header("Prefer", "resolution=merge-duplicates")
                .json(items)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(CiteError::Deploy(format!(
                    "Failed to upsert {table}: HTTP {status} - {body}"
                )));
            }

            eprintln!("    Done");
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

    for table in TABLES {
        let url = format!("{base_url}/rest/v1/{table}?deployment_id=eq.{deployment_id}");
        let response = client
            .delete(&url)
            .header("apikey", &service_key)
            .header("Authorization", format!("Bearer {service_key}"))
            .header("Prefer", "return=minimal")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            eprintln!("{}", format!("  Failed to rollback {table}: HTTP {status}").red().bold());
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

async fn upload_assets(
    build_dir: &Path,
    client: &reqwest::Client,
    base_url: &str,
    service_key: &str,
    bundle: &mut Value,
) -> Result<(), CiteError> {
    let project_slug = bundle
        .get("project")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    for &content_type in &["news", "podcasts"] {
        if let Some(items) = bundle.get_mut(content_type).and_then(|v| v.as_array_mut()) {
            upload_assets_for_type(items, client, base_url, service_key, &project_slug, build_dir, content_type).await?;
        }
    }

    Ok(())
}

async fn upload_assets_for_type(
    items: &mut [Value],
    client: &reqwest::Client,
    base_url: &str,
    service_key: &str,
    project_slug: &str,
    root: &Path,
    content_type: &str,
) -> Result<(), CiteError> {
    for item in items.iter_mut() {
        let obj = match item.as_object_mut() {
            Some(o) => o,
            None => continue,
        };
        let (file_val, slug) = match (
            obj.get("file").and_then(|v| v.as_str()),
            obj.get("slug").and_then(|v| v.as_str()),
        ) {
            (Some(f), Some(s)) => (f, s),
            _ => continue,
        };
        let local = root.join(file_val);
        let storage_path = format!("{project_slug}/{content_type}/{slug}/{file_val}");
        if let Ok(url) = upload_file(client, base_url, service_key, &storage_path, &local).await {
            obj.insert("file".to_string(), Value::String(url));
        }
    }
    Ok(())
}

async fn upload_file(
    client: &reqwest::Client,
    base_url: &str,
    service_key: &str,
    storage_path: &str,
    local_path: &Path,
) -> Result<String, CiteError> {
    if !local_path.exists() {
        return Err(CiteError::Deploy(format!(
            "Asset file not found: {}",
            local_path.display()
        )));
    }

    let bytes = tokio::fs::read(local_path).await?;
    let ext = local_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let mime = mime_for_extension(ext);

    let url = format!("{base_url}/storage/v1/object/{STORAGE_BUCKET}/{storage_path}");

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {service_key}"))
        .header("Content-Type", mime)
        .body(bytes)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        eprintln!(
            "{}",
            format!("  Failed to upload {storage_path}: HTTP {status}").red().bold()
        );
        eprintln!("    {body}");
        return Err(CiteError::Deploy(format!(
            "Failed to upload {storage_path}: HTTP {status}"
        )));
    }

    let public_url = format!("{base_url}/storage/v1/object/public/{STORAGE_BUCKET}/{storage_path}");
    eprintln!("  Uploaded {} to storage", storage_path);
    Ok(public_url)
}

fn has_assets(bundle: &serde_json::Value) -> bool {
    bundle.get("news")
        .or_else(|| bundle.get("podcasts"))
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty())
}

fn mime_for_extension(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "md" => "text/markdown",
        "rst" => "text/x-rst",
        "bib" | "txt" => "text/plain",
        "json" => "application/json",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}
