//! EdgeQuake - High-Performance RAG with Knowledge Graph
//!
//! This is the main entry point for the EdgeQuake server.

mod namespace_factory;

use chrono::{Duration, Utc};
use edgequake_api::{AppState, DocumentTaskProcessor, Server, ServerConfig, StorageMode};
use edgequake_tasks::{
    Pagination, TaskFilter, TaskQueue, TaskStatus, TaskStorage, WorkerPool, WorkerPoolConfig,
};
use std::sync::Arc;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Print the EdgeQuake startup banner with storage mode information.
fn print_startup_banner(version: &str, storage_mode: &StorageMode, host: &str, port: u16) {
    let storage_label = match storage_mode {
        StorageMode::Memory => "MEMORY (ephemeral - data lost on restart)",
        StorageMode::PostgreSQL => "POSTGRESQL (persistent)",
    };

    let storage_icon = match storage_mode {
        StorageMode::Memory => "[M]",
        StorageMode::PostgreSQL => "[P]",
    };

    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("                                                               ");
    println!("   EdgeQuake v{:<47} ", version);
    println!("                                                               ");
    println!("   {} Storage: {:<40} ", storage_icon, storage_label);
    println!("   Server:  http://{}:{:<35} ", host, port);
    println!(
        "   Swagger: http://{}:{}/swagger-ui/{:<20} ",
        host, port, ""
    );
    println!("                                                               ");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
}

/// Recover orphaned tasks that were stuck in "processing" state when backend restarted.
///
/// ## WHY This Fix Is Critical
///
/// When the backend restarts (crash, deployment, manual restart), active tasks remain
/// in "processing" state in the database. Since workers are stateless and don't persist
/// which tasks they were processing, these tasks become orphaned:
/// - They never complete
/// - Workers don't pick them up (status != "pending")
/// - UI shows "Converting PDF: 100%" forever
/// - Users see documents stuck, can't retry without manual DB intervention
///
/// ## Recovery Strategy
///
/// Tasks stuck in "processing" for >5 minutes are assumed orphaned:
/// - Mark as "failed" with clear error message
/// - Users can see the failure and retry manually
/// - Prevents silent data loss and UI confusion
///
/// ## False Positive Risk
///
/// Could mark legitimately slow tasks as failed if they take >5 minutes.
/// Mitigation: 5 minutes is conservative - most docs process in <1 minute.
/// Users can retry immediately if this happens.
///
/// @implements PRODUCTION_BUG_FIX: Orphaned task recovery on startup
async fn recover_orphaned_tasks(
    task_storage: Arc<dyn TaskStorage>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("🔍 Checking for orphaned tasks from previous backend session...");

    let filter = TaskFilter {
        status: Some(TaskStatus::Processing),
        ..Default::default()
    };

    let pagination = Pagination {
        page: 1,
        page_size: 1000, // Process up to 1000 orphaned tasks
        ..Default::default()
    };

    let task_list = task_storage.list_tasks(filter, pagination).await?;
    let now = Utc::now();
    let orphan_threshold = Duration::minutes(5);

    let mut recovered_count = 0;
    let mut skipped_count = 0;

    for mut task in task_list.tasks {
        let age = now.signed_duration_since(task.updated_at);

        if age > orphan_threshold {
            // Task is orphaned - mark as failed
            task.status = TaskStatus::Failed;
            task.error_message = Some(format!(
                "Task orphaned after backend restart. Last updated {} minutes ago. Please retry.",
                age.num_minutes()
            ));
            task.completed_at = Some(now);
            task.updated_at = now;

            match task_storage.update_task(&task).await {
                Ok(_) => {
                    info!(
                        "✅ Recovered orphaned task: {} (age: {} minutes)",
                        task.track_id,
                        age.num_minutes()
                    );
                    recovered_count += 1;
                }
                Err(e) => {
                    warn!(
                        "⚠️ Failed to recover orphaned task {}: {}",
                        task.track_id, e
                    );
                }
            }
        } else {
            // Task is recent - might still be actively processing
            skipped_count += 1;
        }
    }

    if recovered_count > 0 {
        info!(
            "🔧 Orphaned task recovery complete: {} recovered, {} skipped (too recent)",
            recovered_count, skipped_count
        );
    } else if recovered_count == 0 && skipped_count == 0 {
        info!("✅ No orphaned tasks found - clean startup");
    } else {
        info!(
            "✅ All {} processing tasks are recent (<5 min) - likely still active",
            skipped_count
        );
    }

    Ok(())
}

/// Recover orphaned documents stuck in non-terminal states after backend restart.
///
/// ## WHY This Fix Is Critical
///
/// When the backend restarts during upload or processing, documents can remain
/// in non-terminal states like "uploading", "converting", "pending", "processing"
/// in KV storage. Users cannot cancel or reprocess these "stuck" documents because:
/// - The upload/processing context is lost on restart
/// - The cancel endpoint may fail (no matching task, wrong status)
/// - UI shows documents permanently stuck with animated spinners
///
/// ## Recovery Strategy
///
/// Documents with non-terminal status/current_stage updated >5 minutes ago are
/// marked as "failed" with a clear message. Users can then retry or delete them.
///
/// @implements FIX: Stuck uploading status after cancel or server restart
async fn recover_orphaned_documents(
    kv_storage: Arc<dyn edgequake_storage::traits::KVStorage>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("🔍 Checking for orphaned documents from previous backend session...");

    let all_keys = kv_storage.keys().await?;
    let metadata_keys: Vec<String> = all_keys
        .iter()
        .filter(|k| k.ends_with("-metadata"))
        .cloned()
        .collect();

    if metadata_keys.is_empty() {
        info!("✅ No documents found - clean startup");
        return Ok(());
    }

    let metadata_values = kv_storage.get_by_ids(&metadata_keys).await?;
    let now = Utc::now();
    let orphan_threshold = Duration::minutes(5);

    let non_terminal_statuses = [
        "uploading",
        "converting",
        "preprocessing",
        "chunking",
        "extracting",
        "gleaning",
        "merging",
        "summarizing",
        "embedding",
        "storing",
        "pending",
        "processing",
    ];

    let mut recovered_count = 0;

    for (key, value) in metadata_keys.iter().zip(metadata_values.iter()) {
        if let Some(obj) = value.as_object() {
            // Check both `status` and `current_stage` for stuck states
            let status = obj.get("status").and_then(|v| v.as_str()).unwrap_or("");
            let current_stage = obj
                .get("current_stage")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let is_stuck = non_terminal_statuses.contains(&status)
                || non_terminal_statuses.contains(&current_stage);

            if !is_stuck {
                continue;
            }

            // Check age - only recover if old enough to be considered orphaned
            let updated_at = obj
                .get("updated_at")
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc));

            let is_old_enough = match updated_at {
                Some(dt) => now.signed_duration_since(dt) > orphan_threshold,
                // No updated_at → assume orphaned (conservative)
                None => true,
            };

            if !is_old_enough {
                continue;
            }

            // Mark document as failed
            let mut updated = obj.clone();
            updated.insert("status".to_string(), serde_json::json!("failed"));
            updated.insert("current_stage".to_string(), serde_json::json!("failed"));
            updated.insert(
                "stage_message".to_string(),
                serde_json::json!(format!(
                    "Document was stuck in '{}' state after backend restart. Please retry.",
                    if !current_stage.is_empty() {
                        current_stage
                    } else {
                        status
                    }
                )),
            );
            updated.insert(
                "error_message".to_string(),
                serde_json::json!("Orphaned during backend restart - please retry"),
            );
            updated.insert(
                "updated_at".to_string(),
                serde_json::json!(now.to_rfc3339()),
            );

            match kv_storage
                .upsert(&[(key.clone(), serde_json::json!(updated))])
                .await
            {
                Ok(_) => {
                    info!(
                        "✅ Recovered orphaned document: {} (was stuck in '{}')",
                        key,
                        if !current_stage.is_empty() {
                            current_stage
                        } else {
                            status
                        }
                    );
                    recovered_count += 1;
                }
                Err(e) => {
                    warn!("⚠️ Failed to recover orphaned document {}: {}", key, e);
                }
            }
        }
    }

    if recovered_count > 0 {
        info!(
            "🔧 Orphaned document recovery complete: {} recovered",
            recovered_count
        );
    } else {
        info!("✅ No orphaned documents found - clean startup");
    }

    Ok(())
}

/// Requeue pending tasks from database to in-memory queue on startup.
///
/// @implements PRODUCTION_BUG_FIX: Pending task recovery
///
/// ## WHY this is needed
///
/// The worker pool pulls tasks from an in-memory TaskQueue (mpsc channel), not from
/// the database. When tasks are created via API:
/// 1. Task is saved to database with status="pending"
/// 2. Task is enqueued to in-memory TaskQueue
/// 3. Workers pull from TaskQueue and process
///
/// **Problem:** When backend restarts, the in-memory queue is empty! Pending tasks
/// in the database are never picked up by workers.
///
/// **Solution:** On startup, query all pending tasks from database and re-enqueue them
/// to the TaskQueue so workers can process them.
///
/// ## Strategy
///
/// - Query ALL tasks with status="pending" (no age threshold - all pending tasks should be processed)
/// - Enqueue each task to the TaskQueue
/// - Log requeue statistics for visibility
/// - Non-fatal: If requeue fails, warning is logged but startup continues
///
/// ## Ordering
///
/// This MUST run BEFORE starting the worker pool to ensure pending tasks are available
/// when workers start polling.
///
/// ## Risk mitigation
///
/// - Idempotent: Re-enqueueing the same task multiple times is safe (workers dedup)
/// - No race conditions: Workers haven't started yet when this runs
/// - Non-blocking: Uses queue.send() which is async and won't block startup
async fn requeue_pending_tasks(
    task_storage: Arc<dyn TaskStorage>,
    task_queue: Arc<dyn TaskQueue>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("🔄 Checking for pending tasks to requeue from database...");

    // Query all pending tasks
    let filter = TaskFilter {
        status: Some(TaskStatus::Pending),
        ..Default::default()
    };
    let pagination = Pagination {
        page_size: 1000, // WHY 1000: Most deployments won't have >1000 pending tasks at once
        ..Default::default()
    };

    let task_list = task_storage.list_tasks(filter, pagination).await?;
    let pending_count = task_list.tasks.len();

    if pending_count == 0 {
        info!("✅ No pending tasks to requeue");
        return Ok(());
    }

    info!(
        "📋 Found {} pending task(s) in database, requeueing to worker pool...",
        pending_count
    );

    let mut requeued_count = 0;
    let mut failed_count = 0;

    for task in task_list.tasks {
        match task_queue.send(task.clone()).await {
            Ok(_) => {
                info!("✅ Requeued task: {}", task.track_id);
                requeued_count += 1;
            }
            Err(e) => {
                warn!("⚠️ Failed to requeue task {}: {}", task.track_id, e);
                failed_count += 1;
            }
        }
    }

    info!(
        "🔧 Pending task requeue complete: {} requeued, {} failed",
        requeued_count, failed_count
    );

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "edgequake=debug,edgequake_query=debug,edgequake_api=debug,edgequake_core=debug,edgequake_storage=debug,edgequake_llm=debug,edgequake_pipeline=debug,edgequake_tasks=debug,tower_http=debug,axum=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting EdgeQuake v{}", env!("CARGO_PKG_VERSION"));

    // Get API key from environment (optional - Ollama doesn't need it)
    let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();

    // DATABASE_URL is optional: when set, use PostgreSQL for persistent operational
    // storage (tasks, workspaces, conversations). When absent, use in-memory storage
    // which is sufficient for the namespace-scoped entity browser (Neptune/S3/DynamoDB).
    let database_url = std::env::var("DATABASE_URL").ok();
    let has_postgres = database_url.is_some();

    let state = if let Some(ref db_url) = database_url {
        info!("PostgreSQL storage mode (DATABASE_URL detected)");
        AppState::new_postgres(db_url, &api_key)
            .await
            .expect("Failed to initialize PostgreSQL storage")
    } else {
        info!("Memory storage mode (no DATABASE_URL) - namespace entity browser via AWS stores");
        AppState::new_memory(Some(&api_key)).await
    };

    // Construct namespace registry if NAMESPACE_TABLE is set (or use default)
    let namespace_table = std::env::var("NAMESPACE_TABLE")
        .unwrap_or_else(|_| "edgequake-namespaces".to_string());
    let state = {
        use edgequake_storage_aws::{DynamoNamespaceConfig, DynamoNamespaceRegistry};
        let aws_config = edgequake_storage_aws::aws_config::load_defaults(
            edgequake_storage_aws::aws_config::BehaviorVersion::latest(),
        )
        .await;
        let dynamo_client = edgequake_storage_aws::aws_sdk_dynamodb::Client::new(&aws_config);
        let ns_config = DynamoNamespaceConfig {
            table_name: namespace_table.clone(),
        };
        // Construct namespace storage factory for namespace-scoped data operations
        let neptune_endpoint = std::env::var("NEPTUNE_ENDPOINT")
            .unwrap_or_else(|_| "localhost:8182".to_string());
        let vector_bucket = std::env::var("VECTOR_BUCKET")
            .unwrap_or_else(|_| "edgequake-vectors".to_string());
        let embedding_dim: usize = std::env::var("EMBEDDING_DIMENSION")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1536);
        let kv_table = std::env::var("DYNAMODB_TABLE")
            .or_else(|_| std::env::var("PIPELINE_STATE_TABLE"))
            .unwrap_or_else(|_| "edgequake-kv".to_string());

        // Build InfrastructureConfig for MCP descriptor auto-generation
        let infra_config = edgequake_core::InfrastructureConfig {
            neptune_endpoint: neptune_endpoint.clone(),
            vector_bucket_name: vector_bucket.clone(),
            dynamodb_table_name: kv_table.clone(),
            account_id: std::env::var("AWS_ACCOUNT_ID")
                .unwrap_or_else(|_| "000000000000".to_string()),
            region: std::env::var("AWS_REGION")
                .unwrap_or_else(|_| "us-east-1".to_string()),
            environment: std::env::var("ENVIRONMENT")
                .unwrap_or_else(|_| "dev".to_string()),
            external_id_suffix: std::env::var("EXTERNAL_ID_SUFFIX")
                .unwrap_or_else(|_| "default".to_string()),
            athena_bm25_database: std::env::var("ATHENA_BM25_DATABASE").ok(),
            bm25_s3_bucket: std::env::var("BM25_S3_BUCKET").ok(),
            athena_workgroup: std::env::var("ATHENA_WORKGROUP").ok(),
        };

        let registry = DynamoNamespaceRegistry::new(ns_config, dynamo_client)
            .with_infra(infra_config);
        info!(
            table = %namespace_table,
            "Namespace registry configured (DynamoDB, MCP descriptor auto-generation enabled)"
        );

        let factory = namespace_factory::AwsNamespaceStorageFactory::new(
            neptune_endpoint.clone(),
            vector_bucket.clone(),
            embedding_dim,
            kv_table.clone(),
        );
        info!(
            neptune_endpoint = %neptune_endpoint,
            vector_bucket = %vector_bucket,
            embedding_dim = embedding_dim,
            kv_table = %kv_table,
            "Namespace storage factory configured (AWS)"
        );

        let state = state
            .with_namespace_registry(std::sync::Arc::new(registry))
            .with_namespace_storage_factory(std::sync::Arc::new(factory));

        // Construct BM25 storage factory if Athena BM25 env vars are set.
        // When ATHENA_BM25_DATABASE is present, hybrid/BM25 retrieval modes are
        // enabled. When absent, queries fall back to vector-only (backward compatible).
        if let Ok(athena_bm25_database) = std::env::var("ATHENA_BM25_DATABASE") {
            let bm25_s3_bucket = std::env::var("BM25_S3_BUCKET")
                .expect("BM25_S3_BUCKET required when ATHENA_BM25_DATABASE is set");
            let athena_workgroup = std::env::var("ATHENA_WORKGROUP")
                .unwrap_or_else(|_| "primary".to_string());
            let athena_output_location = std::env::var("ATHENA_OUTPUT_LOCATION")
                .expect("ATHENA_OUTPUT_LOCATION required when ATHENA_BM25_DATABASE is set");

            let bm25_factory = namespace_factory::AwsBm25StorageFactory::new(
                athena_bm25_database.clone(),
                bm25_s3_bucket.clone(),
                athena_workgroup.clone(),
                athena_output_location.clone(),
            );
            info!(
                database = %athena_bm25_database,
                s3_bucket = %bm25_s3_bucket,
                workgroup = %athena_workgroup,
                "BM25 storage factory configured (Athena/Iceberg)"
            );
            state.with_bm25_storage_factory(std::sync::Arc::new(bm25_factory))
        } else {
            info!("ATHENA_BM25_DATABASE not set, BM25/hybrid retrieval disabled");
            state
        }
    };

    // Initialize default tenant and workspace for non-authenticated mode
    if let Err(e) = state.initialize_defaults().await {
        tracing::warn!("Failed to initialize defaults: {}", e);
    }

    // Create document task processor with workspace-specific pipeline support (SPEC-032)
    // OODA-223: Strict mode enforces workspace isolation.
    // OODA-10: Also attach progress broadcaster for WebSocket event delivery.
    info!(
        "Using {} workspace isolation mode",
        if has_postgres { "STRICT (PostgreSQL)" } else { "MEMORY" }
    );
    let mut processor = DocumentTaskProcessor::with_workspace_support_strict(
        Arc::clone(&state.pipeline),
        Arc::clone(&state.llm_provider),
        Arc::clone(&state.kv_storage),
        Arc::clone(&state.vector_storage),
        Arc::clone(&state.vector_registry),
        Arc::clone(&state.graph_storage),
        state.pipeline_state.clone(),
        Arc::clone(&state.workspace_service),
        Arc::clone(&state.models_config),
    )
    .with_progress_broadcaster(state.progress_broadcaster.clone());

    // CRITICAL: Attach PDF storage for PDF processing tasks
    if let Some(ref pdf_storage) = state.pdf_storage {
        processor = processor.with_pdf_storage(Arc::clone(pdf_storage));
        info!("📄 PDF storage attached to task processor");
    }

    // Phase 24 D-05: Attach namespace registry so rebuild_kg_* track
    // completion triggers an automatic snapshot export (fail-open).
    if let Some(ref namespace_registry) = state.namespace_registry {
        processor = processor.with_namespace_registry(Arc::clone(namespace_registry));
        info!("Namespace registry attached to task processor (snapshot auto-export enabled)");
    }

    let processor = Arc::new(processor);

    // Configure worker pool
    let worker_config = WorkerPoolConfig {
        num_workers: std::env::var("WORKER_THREADS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| num_cpus::get().max(2)),
        auto_retry: true,
        initial_retry_delay_ms: 5000,
        max_retry_delay_ms: 60000,
        backoff_multiplier: 2.0,
    };

    // Recovery functions only apply to PostgreSQL mode (persistent tasks/documents).
    // In memory mode, there's nothing to recover from a previous session.
    if has_postgres {
        // Recover orphaned tasks from previous backend session (PRODUCTION_BUG_FIX)
        // MUST run BEFORE starting workers to prevent race conditions
        if let Err(e) =
            recover_orphaned_tasks(Arc::clone(&state.task_storage) as Arc<dyn TaskStorage>).await
        {
            warn!("Failed to recover orphaned tasks (non-fatal): {}", e);
        }

        // Recover orphaned documents stuck in non-terminal states (uploading, pending, etc.)
        // MUST run BEFORE starting workers to avoid race with new uploads
        if let Err(e) = recover_orphaned_documents(
            Arc::clone(&state.kv_storage) as Arc<dyn edgequake_storage::traits::KVStorage>,
        )
        .await
        {
            warn!("Failed to recover orphaned documents (non-fatal): {}", e);
        }

        // Requeue pending tasks from database to in-memory queue (PRODUCTION_BUG_FIX)
        // MUST run BEFORE starting workers so tasks are available when workers start polling
        if let Err(e) = requeue_pending_tasks(
            Arc::clone(&state.task_storage) as Arc<dyn TaskStorage>,
            Arc::clone(&state.task_queue) as Arc<dyn TaskQueue>,
        )
        .await
        {
            warn!("Failed to requeue pending tasks (non-fatal): {}", e);
        }
    } else {
        info!("Skipping task/document recovery (memory mode - no persistent state)");
    }

    // Create and start worker pool
    let mut worker_pool = WorkerPool::new(
        worker_config.clone(),
        Arc::clone(&state.task_queue) as Arc<dyn edgequake_tasks::TaskQueue>,
        Arc::clone(&state.task_storage) as Arc<dyn edgequake_tasks::TaskStorage>,
        processor,
    );

    info!(
        "Starting worker pool with {} workers",
        worker_config.num_workers
    );
    worker_pool.start();

    // Configure server
    let config = ServerConfig {
        host: std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
        port: std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080),
        enable_cors: true,
        enable_compression: true,
        enable_swagger: true,
    };

    // Print startup banner with storage mode
    print_startup_banner(
        env!("CARGO_PKG_VERSION"),
        &state.storage_mode,
        &config.host,
        config.port,
    );

    // Run server (this blocks until shutdown)
    let server = Server::new(config, state);
    let result = server.run().await;

    // Graceful shutdown of worker pool
    info!("Shutting down worker pool...");
    worker_pool.shutdown().await;

    result?;
    Ok(())
}
