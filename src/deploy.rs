use crate::error::CiteError;
use crate::project::ProjectContext;
use colored::Colorize;
use serde_json::Value;
use std::env;
use tracing::instrument;

#[instrument(skip(ctx), fields(project = %ctx.manifest.project.name, dry_run))]
pub async fn deploy(ctx: &ProjectContext, dry_run: bool) -> Result<(), CiteError> {
    let backend = ctx
        .manifest
        .backend
        .as_ref()
        .ok_or_else(|| CiteError::Deploy("No [backend] section in cite.toml".to_string()))?;

    // Resolve service key: env var > manifest
    let service_key = env::var("CITE_STAGING_SERVICE_KEY")
        .unwrap_or_else(|_| backend.staging_service_key.clone());

    if service_key.is_empty() {
        return Err(CiteError::Deploy(
            "No service key found. Set CITE_STAGING_SERVICE_KEY env var or configure backend.staging_service_key in cite.toml".to_string(),
        ));
    }

    // Read build artifact
    let bundle_path = ctx.build_dir().join("content.json");
    if !bundle_path.exists() {
        return Err(CiteError::Deploy(
            "No build artifact found. Run 'cite-cli build' first.".to_string(),
        ));
    }
    let bundle_str = std::fs::read_to_string(&bundle_path)?;
    let mut bundle: Value = serde_json::from_str(&bundle_str)?;

    let deployment_id = uuid::Uuid::new_v4().to_string();
    let tables = ["artists", "news", "podcasts", "newsletters", "timelines"];

    eprintln!(
        "{}",
        format!("🚀 Deploying with deployment_id: {deployment_id}")
            .cyan()
            .bold()
    );
    if dry_run {
        eprintln!("{}", "  DRY RUN - no data will be sent".yellow().bold());
    }

    for table in &tables {
        if let Some(items) = bundle.get_mut(table).and_then(|v| v.as_array_mut()) {
            if items.is_empty() {
                continue;
            }

            // Inject deployment_id
            for item in items.iter_mut() {
                if let Some(obj) = item.as_object_mut() {
                    obj.insert(
                        "deployment_id".to_string(),
                        Value::String(deployment_id.clone()),
                    );
                }
            }

            eprintln!(
                "  {} {} items",
                "→".cyan(),
                format!("{}: {}", table, items.len()).white()
            );

            if dry_run {
                continue;
            }

            // Perform upsert
            let url = format!(
                "{}/rest/v1/{}",
                backend.staging_url.trim_end_matches('/'),
                table
            );

            let client = reqwest::Client::new();
            let response = client
                .post(&url)
                .header("apikey", &service_key)
                .header("Authorization", format!("Bearer {}", service_key))
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

            eprintln!("    {} Done", "✔".green());
        }
    }

    if !dry_run {
        eprintln!(
            "{} Deployment complete (id: {deployment_id})",
            "✔".green().bold()
        );
    }

    Ok(())
}
