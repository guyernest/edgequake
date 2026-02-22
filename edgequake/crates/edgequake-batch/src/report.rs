//! Post-store verification and final pipeline report.
//!
//! After the store stage completes, queries storage backends (Neptune, S3 Vectors)
//! for actual counts to verify data landed correctly, then prints a detailed
//! final report including per-stage stats, verification counts, failed document
//! summary, and MCP connection info.

use crate::config::BatchConfig;
use crate::state::JobState;
use edgequake_core::McpDescriptor;
use edgequake_storage::traits::{GraphStorage, VectorStorage};
use tracing::{info, warn};

/// Verification counts from storage backends.
///
/// Queried after the store stage to verify data was persisted correctly.
/// Counts are approximate (Neptune bulk load has eventual consistency).
#[derive(Debug, Default)]
pub struct VerificationCounts {
    /// Entity (node) count from Neptune.
    pub entities: usize,
    /// Relationship (edge) count from Neptune.
    pub relationships: usize,
    /// Vector count from S3 Vectors.
    pub vectors: usize,
}

/// Query storage backends for verification counts after the store stage.
///
/// Creates Neptune and S3 Vectors storage instances and queries for counts.
/// Uses `.unwrap_or(0)` for resilience -- if a query fails, the count is 0
/// rather than failing the entire pipeline.
///
/// A 3-second sleep before calling this function is recommended (Neptune
/// bulk load has eventual consistency).
pub async fn verify_stored_data(
    config: &BatchConfig,
    aws_config: &aws_config::SdkConfig,
) -> VerificationCounts {
    let mut counts = VerificationCounts::default();

    // Query Neptune for entity/relationship counts
    if let Some(ref endpoint) = config.neptune_endpoint {
        info!("Querying Neptune for verification counts...");
        let neptune_config = edgequake_storage_aws::NeptuneConfig::new(endpoint.clone())
            .with_namespace(config.namespace.clone());

        match edgequake_storage_aws::NeptuneGraphStorage::new(neptune_config).await {
            Ok(graph) => {
                counts.entities = graph.node_count().await.unwrap_or_else(|e| {
                    warn!(error = %e, "Failed to query Neptune node count");
                    0
                });
                counts.relationships = graph.edge_count().await.unwrap_or_else(|e| {
                    warn!(error = %e, "Failed to query Neptune edge count");
                    0
                });
                info!(
                    entities = counts.entities,
                    relationships = counts.relationships,
                    "Neptune verification counts retrieved"
                );
            }
            Err(e) => {
                warn!(error = %e, "Failed to create Neptune storage for verification");
            }
        }
    }

    // Query S3 Vectors for vector count
    if let Some(ref bucket) = config.vector_bucket {
        info!("Querying S3 Vectors for verification counts...");
        let index_name = config
            .vector_index
            .clone()
            .unwrap_or_else(|| format!("{}-embeddings", config.namespace));
        let vectors_config = edgequake_storage_aws::S3VectorsConfig {
            vector_bucket_name: bucket.clone(),
            index_name,
            dimension: 1536,
            namespace: config.namespace.clone(),
        };
        let s3v_client = edgequake_storage_aws::aws_sdk_s3vectors::Client::new(aws_config);
        let s3v = edgequake_storage_aws::S3VectorsStorage::new_with_client(
            vectors_config,
            s3v_client,
        );
        counts.vectors = s3v.count().await.unwrap_or_else(|e| {
            warn!(error = %e, "Failed to query S3 Vectors count");
            0
        });
        info!(vectors = counts.vectors, "S3 Vectors verification count retrieved");
    }

    counts
}

/// Print the final pipeline report to stdout.
///
/// Includes:
/// - Job identification and duration
/// - Per-stage summary with timing
/// - Verification counts from storage backends (approximate)
/// - Failed document summary (error counts per phase)
/// - MCP connection info from the namespace descriptor
pub fn print_final_report(
    job: &JobState,
    namespace: &str,
    verification: &VerificationCounts,
    descriptor: Option<&McpDescriptor>,
    duration: std::time::Duration,
) {
    use comfy_table::{Cell, Table};

    let duration_str = humantime::format_duration(truncate_duration(duration)).to_string();

    println!();
    println!("=== Pipeline Complete ===");
    println!();
    println!("Job ID:           {}", job.job_id);
    println!("Namespace:        {}", namespace);
    println!("Duration:         {}", duration_str);
    println!();

    // Per-Stage Summary table
    let mut table = Table::new();
    table.set_header(vec!["Stage", "Duration", "Details"]);

    let completion_millis = chrono::Utc::now().timestamp_millis();
    let phase_order = ["preparing", "extracting", "embedding", "storing"];

    for (i, phase_name) in phase_order.iter().enumerate() {
        if let Some(&start) = job.phase_started_at.get(*phase_name) {
            let end = if i + 1 < phase_order.len() {
                job.phase_started_at
                    .get(phase_order[i + 1])
                    .copied()
                    .unwrap_or(completion_millis)
            } else {
                completion_millis
            };
            let phase_duration_ms = (end - start).max(0) as u64;
            let phase_dur =
                humantime::format_duration(std::time::Duration::from_millis(phase_duration_ms))
                    .to_string();

            let details = match *phase_name {
                "preparing" => {
                    let errors = job.errors_per_phase.get("preparing").copied().unwrap_or(0);
                    if errors > 0 {
                        format!(
                            "{} documents -> {} chunks ({} failed)",
                            job.total_documents, job.total_chunks, errors
                        )
                    } else {
                        format!(
                            "{} documents -> {} chunks",
                            job.total_documents, job.total_chunks
                        )
                    }
                }
                "extracting" => {
                    let errors = job.errors_per_phase.get("extracting").copied().unwrap_or(0);
                    if errors > 0 {
                        format!(
                            "{} chunks processed ({} failed)",
                            job.total_chunks, errors
                        )
                    } else {
                        format!("{} chunks processed", job.total_chunks)
                    }
                }
                "embedding" => {
                    format!("{} embeddings generated", job.total_embeddings)
                }
                "storing" => {
                    format!(
                        "Stored to Neptune + S3 Vectors + DynamoDB"
                    )
                }
                _ => String::new(),
            };

            let display_name = match *phase_name {
                "preparing" => "Prepare",
                "extracting" => "Extract",
                "embedding" => "Embed",
                "storing" => "Store",
                _ => phase_name,
            };

            table.add_row(vec![
                Cell::new(display_name),
                Cell::new(phase_dur),
                Cell::new(details),
            ]);
        }
    }

    println!("Per-Stage Summary:");
    println!("{table}");
    println!();

    // Verification counts
    println!("Verification (queried from storage, approximate):");
    if job.phase_started_at.contains_key("storing") {
        println!(
            "  Neptune:        {} entities, {} relationships",
            verification.entities, verification.relationships
        );
        println!("  S3 Vectors:     {} vectors", verification.vectors);
    } else {
        // Pipeline didn't reach store stage
        println!("  (store stage not reached)");
    }
    println!();

    // Pipeline-tracked counts (from extraction)
    println!("Pipeline Counts (from extraction):");
    println!("  Entities:       {}", job.total_entities);
    println!("  Relationships:  {}", job.total_relationships);
    println!("  Embeddings:     {}", job.total_embeddings);
    println!();

    // Failed documents
    let total_errors: usize = job.errors_per_phase.values().sum();
    if total_errors > 0 {
        println!("Errors ({}):", total_errors);
        for (phase, count) in &job.errors_per_phase {
            if *count > 0 {
                println!("  {}: {} failures", phase, count);
            }
        }
        println!();
    }

    // MCP Connection Info
    if let Some(desc) = descriptor {
        println!("--- MCP Connection Info ---");
        println!("Namespace:        {}", desc.namespace.slug);
        println!(
            "Neptune:          {}",
            desc.storage.neptune.endpoint
        );
        println!(
            "S3 Vectors:       {} / {}",
            desc.storage.s3_vectors.bucket_name, desc.storage.s3_vectors.index_name
        );
        println!(
            "DynamoDB:         {}",
            desc.storage.dynamodb.table_name
        );
    } else {
        println!("--- MCP Connection Info ---");
        println!(
            "MCP descriptor not found for this namespace."
        );
        println!(
            "Create one via the management console to enable MCP server queries."
        );
    }
    println!("===========================");
    println!();
}

/// Truncate a duration to the nearest second for display.
///
/// Removes sub-second precision for cleaner output (e.g., "3m 42s" instead of "3m 42s 123ms 456us").
fn truncate_duration(d: std::time::Duration) -> std::time::Duration {
    std::time::Duration::from_secs(d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{JobState, Phase, StateManager};

    #[test]
    fn test_verification_counts_default() {
        let counts = VerificationCounts::default();
        assert_eq!(counts.entities, 0);
        assert_eq!(counts.relationships, 0);
        assert_eq!(counts.vectors, 0);
    }

    #[test]
    fn test_print_final_report_no_descriptor() {
        // Smoke test: ensure print_final_report doesn't panic with no descriptor
        let job = StateManager::create_job("test-report-job");
        let counts = VerificationCounts {
            entities: 127,
            relationships: 89,
            vectors: 996,
        };
        // This will print to stdout -- just verify no panic
        print_final_report(
            &job,
            "test-ns",
            &counts,
            None,
            std::time::Duration::from_secs(222),
        );
    }

    #[test]
    fn test_print_final_report_with_descriptor() {
        let mut job = StateManager::create_job("test-report-job");
        job.total_documents = 42;
        job.total_chunks = 168;
        job.total_entities = 127;
        job.total_relationships = 89;
        job.total_embeddings = 996;
        job.phase = Phase::Completed;
        job.phase_started_at.insert("preparing".to_string(), 1000000);
        job.phase_started_at.insert("extracting".to_string(), 1004200);
        job.phase_started_at.insert("embedding".to_string(), 1155300);
        job.phase_started_at.insert("storing".to_string(), 1173600);

        let counts = VerificationCounts {
            entities: 127,
            relationships: 89,
            vectors: 996,
        };

        let descriptor = McpDescriptor {
            schema_version: "1.0".to_string(),
            namespace: edgequake_core::McpNamespaceInfo {
                slug: "test-ns".to_string(),
                description: Some("Test namespace".to_string()),
            },
            storage: edgequake_core::McpStorageConfig {
                neptune: edgequake_core::McpNeptuneConfig {
                    endpoint: "neptune.us-east-1.neptune.amazonaws.com:8182".to_string(),
                    label_prefix: "test-ns".to_string(),
                },
                s3_vectors: edgequake_core::McpS3VectorsConfig {
                    bucket_name: "edgequake-vectors".to_string(),
                    index_name: "test-ns-embeddings".to_string(),
                },
                dynamodb: edgequake_core::McpDynamoDbConfig {
                    table_name: "edgequake-kv".to_string(),
                    namespace_key: "test-ns".to_string(),
                },
            },
            auth: edgequake_core::McpAuthConfig {
                role_arn: "arn:aws:iam::123456789012:role/test".to_string(),
                external_id: "eq-test-abc".to_string(),
                region: "us-east-1".to_string(),
            },
            pipeline_config: edgequake_core::McpPipelineConfig {
                embedding_model: "text-embedding-3-small".to_string(),
                embedding_dimension: 1536,
                llm_model: "gpt-4o-mini".to_string(),
            },
            tools: vec![],
            generated_at: 1700000000000,
        };

        print_final_report(
            &job,
            "test-ns",
            &counts,
            Some(&descriptor),
            std::time::Duration::from_secs(222),
        );
    }

    #[test]
    fn test_print_final_report_with_errors() {
        let mut job = StateManager::create_job("test-report-errors");
        job.total_documents = 42;
        job.total_chunks = 168;
        job.phase = Phase::Completed;
        job.errors_per_phase.insert("preparing".to_string(), 2);
        job.errors_per_phase.insert("extracting".to_string(), 1);

        let counts = VerificationCounts::default();
        print_final_report(
            &job,
            "test-ns",
            &counts,
            None,
            std::time::Duration::from_secs(60),
        );
    }

    #[test]
    fn test_truncate_duration() {
        let d = std::time::Duration::from_millis(3_742_123);
        let truncated = truncate_duration(d);
        assert_eq!(truncated, std::time::Duration::from_secs(3742));
    }
}
