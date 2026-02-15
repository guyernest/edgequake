//! Neptune bulk load via S3.
//!
//! Generates Gremlin CSV files, uploads them to S3, and triggers Neptune's
//! bulk loader API. This avoids the Gremlin WebSocket SigV4 signing issue
//! and is faster for batch ingestion.

use crate::config::BatchConfig;
use crate::progress::BatchProgress;
use anyhow::{bail, Context};
use aws_sdk_neptunedata::types::{Format, Parallelism};
use edgequake_pipeline::extractor::ExtractionResult;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, warn};

/// Run Neptune bulk load: generate CSVs, upload to S3, trigger loader, poll.
pub async fn run_bulk_load(
    config: &BatchConfig,
    aws_config: &aws_config::SdkConfig,
    extractions: &HashMap<String, ExtractionResult>,
    progress: &BatchProgress,
) -> anyhow::Result<()> {
    let s3_bucket = config
        .s3_bucket
        .as_ref()
        .context("--s3-bucket is required for Neptune bulk load")?;
    let neptune_role_arn = config
        .neptune_role_arn
        .as_ref()
        .context("--neptune-role-arn is required for Neptune bulk load")?;
    let neptune_endpoint = config
        .neptune_endpoint
        .as_ref()
        .context("--neptune-endpoint is required for Neptune bulk load")?;

    let job_id = &config.job_id;

    // Step 1: Generate Gremlin CSV files
    info!("Generating Gremlin CSV files for bulk load");
    let bulk_dir = config.bulk_load_dir();
    std::fs::create_dir_all(&bulk_dir)?;

    let (vertex_count, edge_count) =
        generate_csv(extractions, &bulk_dir, &config.namespace, progress)?;
    info!(
        vertices = vertex_count,
        edges = edge_count,
        "CSV generation complete"
    );

    if vertex_count == 0 {
        warn!("No vertices to load, skipping bulk load");
        return Ok(());
    }

    // Step 2: Upload CSVs to S3
    info!(bucket = %s3_bucket, "Uploading CSV files to S3");
    let s3_client = aws_sdk_s3::Client::new(aws_config);
    let s3_prefix = format!("bulk-load/{}", job_id);

    upload_to_s3(
        &s3_client,
        s3_bucket,
        &s3_prefix,
        &bulk_dir.join("vertices.csv"),
        "vertices.csv",
    )
    .await?;

    if edge_count > 0 {
        upload_to_s3(
            &s3_client,
            s3_bucket,
            &s3_prefix,
            &bulk_dir.join("edges.csv"),
            "edges.csv",
        )
        .await?;
    }
    info!("S3 upload complete");

    // Step 3: Build Neptune client with assumed-role credentials
    let neptune_client = build_neptune_client(aws_config, neptune_endpoint).await?;

    let region = aws_config
        .region()
        .map(|r| r.to_string())
        .unwrap_or_else(|| "us-east-1".to_string());

    // Step 4: Start vertices load
    info!("Starting Neptune bulk load for vertices");
    let vertices_source = format!("s3://{}/{}/vertices.csv", s3_bucket, s3_prefix);
    let vertices_load_id = start_load(
        &neptune_client,
        &vertices_source,
        neptune_role_arn,
        &region,
        None,
    )
    .await?;
    info!(load_id = %vertices_load_id, "Vertices load job started");

    // Poll vertices load to completion
    poll_load(&neptune_client, &vertices_load_id, "vertices").await?;

    // Step 5: Start edges load (depends on vertices completing first)
    if edge_count > 0 {
        info!("Starting Neptune bulk load for edges");
        let edges_source = format!("s3://{}/{}/edges.csv", s3_bucket, s3_prefix);
        let edges_load_id = start_load(
            &neptune_client,
            &edges_source,
            neptune_role_arn,
            &region,
            None,
        )
        .await?;
        info!(load_id = %edges_load_id, "Edges load job started");

        poll_load(&neptune_client, &edges_load_id, "edges").await?;
    }

    info!(
        vertices = vertex_count,
        edges = edge_count,
        "Neptune bulk load complete"
    );

    Ok(())
}

/// Score a description by investigation relevance.
///
/// Higher scores = more relevant to the Epstein investigation.
/// Used to rank descriptions when aggregating entities across chunks,
/// so the best description is shown first in the merged output.
fn score_description(desc: &str) -> i32 {
    let lower = desc.to_lowercase();
    let mut score: i32 = 0;

    // Base score: prefer longer, more informative descriptions
    score += (desc.len().min(200) / 10) as i32;

    // Bonus for investigation-relevant keywords
    const INVESTIGATION_TERMS: &[&str] = &[
        "epstein",
        "maxwell",
        "trafficking",
        "abuse",
        "victim",
        "allegation",
        "indictment",
        "prosecution",
        "deposition",
        "testimony",
        "witness",
        "defendant",
        "attorney",
        "counsel",
        "plea",
        "settlement",
        "subpoena",
        "investigation",
        "fbi",
        "doj",
        "grand jury",
        "conspiracy",
        "obstruction",
        "flight log",
        "palm beach",
        "little st. james",
        "non-prosecution",
        "recruited",
        "massage",
        "minor",
        "underage",
        "foundation",
        "wire transfer",
        "financial",
        "trust fund",
    ];
    for term in INVESTIGATION_TERMS {
        if lower.contains(term) {
            score += 5;
        }
    }

    // Bonus for timestamps (indicates well-grounded facts)
    if lower.contains("[timestamp:") {
        score += 10;
    }

    // Penalty for generic/irrelevant descriptions
    const IRRELEVANT_INDICATORS: &[&str] = &[
        "polling average",
        "variable in",
        "currency",
        "exchange rate",
        "stock price",
        "weather",
        "sports",
        "recipe",
        "fictional",
    ];
    for term in IRRELEVANT_INDICATORS {
        if lower.contains(term) {
            score -= 20;
        }
    }

    // Penalty for very short descriptions (likely unhelpful)
    if desc.len() < 20 {
        score -= 5;
    }

    // Penalty for "Also known as" only descriptions (no real content)
    if lower.starts_with("also known as") {
        score -= 10;
    }

    score
}

/// Generate Gremlin CSV files for vertices and edges.
fn generate_csv(
    extractions: &HashMap<String, ExtractionResult>,
    output_dir: &PathBuf,
    namespace: &str,
    progress: &BatchProgress,
) -> anyhow::Result<(usize, usize)> {
    // -- Vertices CSV --
    let vertices_path = output_dir.join("vertices.csv");
    let mut vertex_writer = csv::Writer::from_path(&vertices_path)?;

    // Header: ~id, ~label, property:Type columns
    vertex_writer.write_record([
        "~id",
        "~label",
        "entity_type:String",
        "description:String",
        "namespace:String",
        "source_chunk_ids:String",
        "source_document_id:String",
        "source_file_path:String",
    ])?;

    // Aggregate entities by name across all extractions.
    // Same entity may appear in multiple chunks — we merge descriptions,
    // source chunk IDs, and keep the most specific entity_type.
    struct AggregatedEntity {
        entity_type: String,
        descriptions: Vec<String>,
        source_chunk_ids: Vec<String>,
        source_document_id: Option<String>,
        source_file_path: Option<String>,
    }

    let mut entity_map: HashMap<String, AggregatedEntity> = HashMap::new();
    let total_entities: usize = extractions.values().map(|r| r.entities.len()).sum();
    let bar = progress.document_bar(total_entities as u64, "Aggregating entities");

    for result in extractions.values() {
        for entity in &result.entities {
            let entry = entity_map
                .entry(entity.name.clone())
                .or_insert_with(|| AggregatedEntity {
                    entity_type: entity.entity_type.clone(),
                    descriptions: Vec::new(),
                    source_chunk_ids: Vec::new(),
                    source_document_id: entity.source_document_id.clone(),
                    source_file_path: entity.source_file_path.clone(),
                });

            // Prefer a non-UNKNOWN entity type
            if entry.entity_type == "UNKNOWN" && entity.entity_type != "UNKNOWN" {
                entry.entity_type = entity.entity_type.clone();
            }

            // Collect unique descriptions (skip empty and duplicates)
            let desc = entity.description.trim();
            if !desc.is_empty() && !entry.descriptions.iter().any(|d| d == desc) {
                entry.descriptions.push(desc.to_string());
            }

            // Merge source chunk IDs
            for chunk_id in &entity.source_chunk_ids {
                if !entry.source_chunk_ids.contains(chunk_id) {
                    entry.source_chunk_ids.push(chunk_id.clone());
                }
            }

            // Fill in provenance if missing
            if entry.source_document_id.is_none() {
                entry.source_document_id = entity.source_document_id.clone();
            }
            if entry.source_file_path.is_none() {
                entry.source_file_path = entity.source_file_path.clone();
            }

            bar.inc(1);
        }
    }
    bar.finish_with_message(format!("Aggregated {} unique entities", entity_map.len()));

    // Write aggregated entities to CSV
    let mut vertex_count = 0;
    let mut seen_entities: HashMap<String, ()> = HashMap::new();
    for (name, agg) in &entity_map {
        seen_entities.insert(name.clone(), ());

        // Rank descriptions by investigation relevance, take top 3
        let mut scored: Vec<(i32, &String)> = agg
            .descriptions
            .iter()
            .map(|d| (score_description(d), d))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0)); // highest score first
        let merged_description = scored
            .iter()
            .take(3)
            .map(|(_, d)| d.as_str())
            .collect::<Vec<_>>()
            .join(" | ");

        let source_chunk_ids = agg.source_chunk_ids.join("|");
        let source_document_id = agg.source_document_id.as_deref().unwrap_or("");
        let source_file_path = agg.source_file_path.as_deref().unwrap_or("");
        vertex_writer.write_record([
            name.as_str(),
            "Entity",
            &agg.entity_type,
            &merged_description,
            namespace,
            &source_chunk_ids,
            source_document_id,
            source_file_path,
        ])?;
        vertex_count += 1;
    }

    // Auto-create vertices for relationship endpoints not in entity list
    let mut missing_count = 0;
    for result in extractions.values() {
        for rel in &result.relationships {
            for name in [&rel.source, &rel.target] {
                if !seen_entities.contains_key(name) {
                    seen_entities.insert(name.clone(), ());
                    vertex_writer.write_record([
                        name.as_str(),
                        "Entity",
                        "UNKNOWN",
                        "",
                        namespace,
                        "", // source_chunk_ids
                        "", // source_document_id
                        "", // source_file_path
                    ])?;
                    vertex_count += 1;
                    missing_count += 1;
                }
            }
        }
    }

    vertex_writer.flush()?;
    if missing_count > 0 {
        info!(
            count = missing_count,
            "Auto-created vertices for relationship endpoints"
        );
    }
    bar.finish_with_message(format!("Generated {} vertices", vertex_count));

    // -- Edges CSV --
    let edges_path = output_dir.join("edges.csv");
    let mut edge_writer = csv::Writer::from_path(&edges_path)?;

    edge_writer.write_record([
        "~id",
        "~from",
        "~to",
        "~label",
        "description:String",
        "weight:Double",
        "namespace:String",
        "keywords:String",
        "source_chunk_id:String",
        "source_document_id:String",
        "source_file_path:String",
    ])?;

    let total_rels: usize = extractions.values().map(|r| r.relationships.len()).sum();
    let bar = progress.document_bar(total_rels as u64, "Generating edge CSV");

    let mut edge_count = 0;
    let mut seen_edges: HashMap<String, ()> = HashMap::new();
    for result in extractions.values() {
        for rel in &result.relationships {
            let edge_id = format!("{}:{}", rel.source, rel.target);
            if seen_edges.contains_key(&edge_id) {
                bar.inc(1);
                continue;
            }
            seen_edges.insert(edge_id.clone(), ());

            let keywords = rel.keywords.join(", ");
            let source_chunk_id = rel.source_chunk_id.as_deref().unwrap_or("");
            let source_document_id = rel.source_document_id.as_deref().unwrap_or("");
            let source_file_path = rel.source_file_path.as_deref().unwrap_or("");
            edge_writer.write_record([
                &edge_id,
                &rel.source,
                &rel.target,
                "relates_to",
                &rel.description,
                &rel.weight.to_string(),
                namespace,
                &keywords,
                source_chunk_id,
                source_document_id,
                source_file_path,
            ])?;
            edge_count += 1;
            bar.inc(1);
        }
    }

    edge_writer.flush()?;
    bar.finish_with_message(format!("Generated {} edges", edge_count));

    Ok((vertex_count, edge_count))
}

/// Upload a local file to S3.
async fn upload_to_s3(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    prefix: &str,
    local_path: &PathBuf,
    filename: &str,
) -> anyhow::Result<()> {
    let key = format!("{}/{}", prefix, filename);

    // Read file into memory (CSVs are small) to avoid async stream timeouts
    let data =
        std::fs::read(local_path).context(format!("Failed to read {}", local_path.display()))?;
    let content_length = data.len() as i64;
    info!(file = %filename, bytes = content_length, "Uploading to S3");
    let body = aws_sdk_s3::primitives::ByteStream::from(data);

    client
        .put_object()
        .bucket(bucket)
        .key(&key)
        .content_length(content_length)
        .body(body)
        .send()
        .await
        .context(format!(
            "Failed to upload {} to s3://{}/{}",
            filename, bucket, key
        ))?;

    info!(key = %key, "Uploaded to S3");
    Ok(())
}

/// Build a Neptune data client using the endpoint URL.
/// The caller's credentials (from aws_config) must have neptune-db:* permissions
/// — either directly or via an assumed role.
async fn build_neptune_client(
    aws_config: &aws_config::SdkConfig,
    neptune_endpoint: &str,
) -> anyhow::Result<aws_sdk_neptunedata::Client> {
    // Neptune endpoint may or may not include port/scheme.
    // The loader API runs on port 8182 (same as Gremlin/SPARQL).
    let endpoint_url = if neptune_endpoint.starts_with("https://") {
        neptune_endpoint.to_string()
    } else if neptune_endpoint.contains(':') {
        // Already has port (e.g. "host:8182")
        format!("https://{}", neptune_endpoint)
    } else {
        format!("https://{}:8182", neptune_endpoint)
    };

    let neptune_config = aws_sdk_neptunedata::config::Builder::from(aws_config)
        .endpoint_url(&endpoint_url)
        .build();

    Ok(aws_sdk_neptunedata::Client::from_conf(neptune_config))
}

/// Start a Neptune loader job and return the load ID.
async fn start_load(
    client: &aws_sdk_neptunedata::Client,
    source: &str,
    iam_role_arn: &str,
    region: &str,
    dependencies: Option<Vec<String>>,
) -> anyhow::Result<String> {
    let mut req = client
        .start_loader_job()
        .source(source)
        .format(Format::Csv)
        .iam_role_arn(iam_role_arn)
        .s3_bucket_region(region.into())
        .fail_on_error(false)
        .parallelism(Parallelism::Medium)
        .update_single_cardinality_properties(true)
        .queue_request(true);

    if let Some(deps) = dependencies {
        for dep in deps {
            req = req.dependencies(dep);
        }
    }

    let resp = req
        .send()
        .await
        .context("Failed to start Neptune loader job")?;

    // payload() returns &HashMap<String, String> with a "loadId" key
    let load_id = resp
        .payload()
        .get("loadId")
        .ok_or_else(|| anyhow::anyhow!("No loadId in loader response payload"))?
        .clone();

    Ok(load_id)
}

/// Extract a string from a nested Document path like payload.overallStatus.status
fn doc_string<'a>(doc: &'a aws_smithy_types::Document, keys: &[&str]) -> Option<&'a str> {
    let mut current = doc;
    for key in keys {
        current = current.as_object()?.get(*key)?;
    }
    current.as_string()
}

/// Poll a Neptune loader job until completion.
async fn poll_load(
    client: &aws_sdk_neptunedata::Client,
    load_id: &str,
    label: &str,
) -> anyhow::Result<()> {
    let max_polls = 360; // 30 minutes at 5s intervals
    let poll_interval = std::time::Duration::from_secs(5);

    for attempt in 0..max_polls {
        tokio::time::sleep(poll_interval).await;

        let resp = client.get_loader_job_status().load_id(load_id).send().await;

        match resp {
            Ok(status) => {
                // payload() returns &Document (an enum: Object, String, etc.)
                let payload = status.payload();

                let overall_status =
                    doc_string(payload, &["overallStatus", "status"]).unwrap_or("UNKNOWN");

                let total_records =
                    doc_string(payload, &["overallStatus", "totalRecords"]).unwrap_or("?");

                let total_errors =
                    doc_string(payload, &["overallStatus", "errors", "errorCount"]).unwrap_or("0");

                info!(
                    label = label,
                    status = overall_status,
                    records = total_records,
                    errors = total_errors,
                    attempt = attempt + 1,
                    "Load job status"
                );

                match overall_status {
                    "LOAD_COMPLETED" => {
                        info!(label = label, "Load completed successfully");
                        return Ok(());
                    }
                    "LOAD_FAILED" | "LOAD_CANCELLED_BY_USER" => {
                        // Try to fetch detailed error info
                        let detail_resp = client
                            .get_loader_job_status()
                            .load_id(load_id)
                            .errors(true)
                            .errors_per_page(10)
                            .send()
                            .await;

                        if let Ok(detail) = detail_resp {
                            warn!(
                                label = label,
                                payload = ?detail.payload(),
                                "Load failed - detailed payload"
                            );
                        }

                        bail!(
                            "Neptune {} load failed with status: {} (errors: {})",
                            label,
                            overall_status,
                            total_errors,
                        );
                    }
                    _ => {
                        // Still in progress (LOAD_IN_PROGRESS, LOAD_NOT_STARTED, etc.)
                    }
                }
            }
            Err(e) => {
                warn!(
                    label = label,
                    error = %e,
                    attempt = attempt + 1,
                    "Failed to get load status, retrying"
                );
            }
        }
    }

    bail!(
        "Neptune {} load timed out after {} seconds",
        label,
        max_polls * 5
    );
}
