//! DTOs for WebSocket progress streaming.
//!
//! This module contains the core types used for real-time progress streaming
//! via WebSocket connections.

use serde::Serialize;
use tokio::sync::broadcast;
use tracing::warn;

// ============================================================================
// Progress Event Types
// ============================================================================

/// Progress event types sent over WebSocket.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum ProgressEvent {
    /// Pipeline job started.
    JobStarted {
        job_name: String,
        total_documents: u32,
        total_batches: u32,
    },
    /// Document processing progress.
    DocumentProgress {
        document_id: String,
        entities_extracted: usize,
        processed: u32,
        total: u32,
    },
    /// Document processing failed.
    DocumentFailed {
        document_id: String,
        error: String,
        processed: u32,
        total: u32,
    },
    /// Batch completed.
    BatchCompleted { batch: u32, total_batches: u32 },
    /// Pipeline job finished.
    JobFinished {
        total_processed: u32,
        duration_ms: u64,
    },
    /// Pipeline message (info, warn, error).
    Message {
        level: String,
        message: String,
        timestamp: String,
    },
    /// Status snapshot (for initial sync or periodic updates).
    StatusSnapshot {
        is_busy: bool,
        job_name: Option<String>,
        processed_documents: u32,
        total_documents: u32,
        current_batch: u32,
        total_batches: u32,
    },
    /// Heartbeat/ping for connection keepalive.
    Heartbeat { timestamp: String },
    /// Connection established confirmation.
    Connected { message: String },
    /// Cancellation requested.
    CancellationRequested,
    /// Chunk extraction failure notification.
    ///
    /// @implements SPEC-003: Chunk-level resilience with failure visibility
    ///
    /// WHY: When using process_with_resilience, some chunks may fail while
    /// others succeed. This event notifies WebSocket clients about individual
    /// chunk failures for UI display and potential retry.
    ChunkFailure {
        /// Document being processed.
        document_id: String,
        /// Task tracking ID.
        task_id: String,
        /// Failed chunk index (0-based).
        chunk_index: u32,
        /// Total chunks in document.
        total_chunks: u32,
        /// Error message describing the failure.
        error_message: String,
        /// Whether the failure was due to timeout.
        was_timeout: bool,
        /// Number of retry attempts before giving up.
        retry_attempts: u32,
    },
    /// PDF page-level extraction progress.
    ///
    /// @implements SPEC-001-upload-pdf: Page-by-page progress during PDF conversion
    /// @implements OODA-06: Add PdfPageProgress event
    ///
    /// WHY: Large PDFs (30+ pages) take significant time to extract.
    /// This event provides page-by-page feedback so users see continuous progress
    /// instead of a generic "Processing..." message.
    ///
    /// ## Event Lifecycle
    ///
    /// For a 5-page PDF, events arrive in this order:
    /// 1. PdfPageProgress { phase: "start", page_num: 0 } - extraction begins
    /// 2. PdfPageProgress { phase: "extraction", page_num: 1 } - page 1 extracting
    /// 3. PdfPageProgress { phase: "extraction", page_num: 1, success: true }
    /// 4. ... repeat for pages 2-5 ...
    /// 5. PdfPageProgress { phase: "complete", page_num: 5, success: true }
    PdfPageProgress {
        /// PDF document ID.
        pdf_id: String,
        /// Task tracking ID.
        task_id: String,
        /// Current page number (1-indexed, 0 for start/complete events).
        page_num: u32,
        /// Total pages in PDF.
        total_pages: u32,
        /// Current phase: "start", "extraction", "complete".
        phase: String,
        /// Markdown length for this page (0 if not yet rendered).
        markdown_len: usize,
        /// Whether this page completed successfully.
        success: bool,
        /// Error message if this page failed.
        error: Option<String>,
    },
}

// ============================================================================
// Ingestion Completed Event (flat snake_case, Option B parallel channel)
// ============================================================================

/// Summary counts from ingestion processing.
#[derive(Debug, Clone, Serialize)]
pub struct IngestionSummary {
    pub chunks: u64,
    pub entities: u64,
    pub relationships: u64,
    pub total_cost_usd: f64,
}

/// Per-dimension counts from a snapshot export.
#[derive(Debug, Clone, Serialize)]
pub struct IngestionCounts {
    pub documents: u64,
    pub chunks: u64,
    pub entities: u64,
    pub relationships: u64,
    pub vectors: u64,
    pub bm25: u64,
}

/// Snapshot enrichment block, present only when export succeeded.
#[derive(Debug, Clone, Serialize)]
pub struct IngestionSnapshot {
    pub uri: String,
    pub created_at: String,
    pub counts: IngestionCounts,
}

/// Flat snake_case ingestion_completed event emitted over the raw-String
/// broadcast channel. The `"type"` discriminator is emitted as the string
/// literal `"ingestion_completed"` with NO outer `data` envelope — matching
/// the webui's `IngestionCompletedEvent` wire shape exactly.
#[derive(Debug, Clone, Serialize)]
pub struct IngestionCompletedEvent {
    #[serde(rename = "type")]
    pub event_type: String, // always "ingestion_completed"
    pub track_id: String,
    pub document_id: String,
    pub completed_at: String,
    pub total_duration_ms: u64,
    pub summary: IngestionSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<IngestionSnapshot>,
}

impl IngestionCompletedEvent {
    /// Construct the event. The `event_type` field is hardcoded to the
    /// `"ingestion_completed"` literal so callers cannot emit a wrong type.
    pub fn new(
        track_id: String,
        document_id: String,
        completed_at: String,
        total_duration_ms: u64,
        summary: IngestionSummary,
        snapshot: Option<IngestionSnapshot>,
    ) -> Self {
        Self {
            event_type: "ingestion_completed".to_string(),
            track_id,
            document_id,
            completed_at,
            total_duration_ms,
            summary,
            snapshot,
        }
    }
}

// ============================================================================
// Progress Broadcaster
// ============================================================================

/// Broadcast channel for progress events.
///
/// This struct manages the broadcast channel that distributes progress events
/// to all connected WebSocket clients.
///
/// Two parallel channels:
/// - `sender`: typed `ProgressEvent` channel (PascalCase `{type,data}` envelope)
/// - `raw_sender`: raw pre-serialized `String` channel for flat snake_case events
///   (e.g. `ingestion_completed`). Only accessible via `broadcast_ingestion_completed`
///   or through the crate-internal `subscribe_raw`/`broadcast_raw` helpers.
#[derive(Clone)]
pub struct ProgressBroadcaster {
    sender: broadcast::Sender<ProgressEvent>,
    raw_sender: broadcast::Sender<String>,
}

impl Default for ProgressBroadcaster {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl ProgressBroadcaster {
    /// Create a new progress broadcaster with specified channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        let (raw_sender, _) = broadcast::channel(capacity);
        Self { sender, raw_sender }
    }

    /// Subscribe to typed progress events.
    pub fn subscribe(&self) -> broadcast::Receiver<ProgressEvent> {
        self.sender.subscribe()
    }

    /// Subscribe to the raw pre-serialized string channel.
    ///
    /// `pub(crate)` — only `websocket.rs` (same crate) drains this channel.
    /// External callers use `broadcast_ingestion_completed` as the only typed entry.
    pub(crate) fn subscribe_raw(&self) -> broadcast::Receiver<String> {
        self.raw_sender.subscribe()
    }

    /// Send a pre-serialized JSON string over the raw channel (fail-open).
    ///
    /// `pub(crate)` — not part of the public API. Use `broadcast_ingestion_completed`
    /// for typed ingestion events so the caller cannot bypass event construction.
    pub(crate) fn broadcast_raw(&self, json: String) {
        let _ = self.raw_sender.send(json);
    }

    /// Serialize `event` to a flat snake_case JSON string and broadcast it over
    /// the raw channel. The only public entry point for the raw channel.
    ///
    /// Fail-open: serialize errors are logged and the event is silently dropped.
    pub fn broadcast_ingestion_completed(&self, event: IngestionCompletedEvent) {
        match serde_json::to_string(&event) {
            Ok(s) => self.broadcast_raw(s),
            Err(e) => warn!(error = %e, "ingestion_completed serialize failed; dropping"),
        }
    }

    /// Broadcast a progress event to all subscribers.
    pub fn broadcast(&self, event: ProgressEvent) {
        // Ignore send errors (no subscribers)
        let _ = self.sender.send(event);
    }

    /// Broadcast job started event.
    pub fn job_started(&self, job_name: &str, total_documents: u32, total_batches: u32) {
        self.broadcast(ProgressEvent::JobStarted {
            job_name: job_name.to_string(),
            total_documents,
            total_batches,
        });
    }

    /// Broadcast document progress event.
    pub fn document_progress(
        &self,
        document_id: &str,
        entities_extracted: usize,
        processed: u32,
        total: u32,
    ) {
        self.broadcast(ProgressEvent::DocumentProgress {
            document_id: document_id.to_string(),
            entities_extracted,
            processed,
            total,
        });
    }

    /// Broadcast document failed event.
    pub fn document_failed(&self, document_id: &str, error: &str, processed: u32, total: u32) {
        self.broadcast(ProgressEvent::DocumentFailed {
            document_id: document_id.to_string(),
            error: error.to_string(),
            processed,
            total,
        });
    }

    /// Broadcast batch completed event.
    pub fn batch_completed(&self, batch: u32, total_batches: u32) {
        self.broadcast(ProgressEvent::BatchCompleted {
            batch,
            total_batches,
        });
    }

    /// Broadcast job finished event.
    pub fn job_finished(&self, total_processed: u32, duration_ms: u64) {
        self.broadcast(ProgressEvent::JobFinished {
            total_processed,
            duration_ms,
        });
    }

    /// Broadcast a message event.
    pub fn message(&self, level: &str, message: &str) {
        self.broadcast(ProgressEvent::Message {
            level: level.to_string(),
            message: message.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        });
    }

    /// Broadcast cancellation requested event.
    pub fn cancellation_requested(&self) {
        self.broadcast(ProgressEvent::CancellationRequested);
    }

    /// Broadcast chunk failure event.
    ///
    /// @implements SPEC-003: Chunk-level resilience with failure visibility
    ///
    /// WHY: This enables the frontend to show which chunks failed during
    /// document processing and why, supporting the resilient extraction feature.
    #[allow(clippy::too_many_arguments)]
    pub fn broadcast_chunk_failure(
        &self,
        document_id: String,
        task_id: String,
        chunk_index: u32,
        total_chunks: u32,
        error_message: String,
        was_timeout: bool,
        retry_attempts: u32,
    ) {
        self.broadcast(ProgressEvent::ChunkFailure {
            document_id,
            task_id,
            chunk_index,
            total_chunks,
            error_message,
            was_timeout,
            retry_attempts,
        });
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_event_job_started_serialization() {
        let event = ProgressEvent::JobStarted {
            job_name: "test-job".to_string(),
            total_documents: 10,
            total_batches: 2,
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("JobStarted"));
        assert!(json.contains("test-job"));
        assert!(json.contains("10"));
    }

    #[test]
    fn test_progress_event_document_progress_serialization() {
        let event = ProgressEvent::DocumentProgress {
            document_id: "doc-1".to_string(),
            entities_extracted: 5,
            processed: 1,
            total: 10,
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("DocumentProgress"));
        assert!(json.contains("doc-1"));
    }

    #[test]
    fn test_progress_event_document_failed_serialization() {
        let event = ProgressEvent::DocumentFailed {
            document_id: "doc-2".to_string(),
            error: "Parse error".to_string(),
            processed: 2,
            total: 10,
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("DocumentFailed"));
        assert!(json.contains("Parse error"));
    }

    #[test]
    fn test_progress_event_batch_completed_serialization() {
        let event = ProgressEvent::BatchCompleted {
            batch: 1,
            total_batches: 5,
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("BatchCompleted"));
    }

    #[test]
    fn test_progress_event_job_finished_serialization() {
        let event = ProgressEvent::JobFinished {
            total_processed: 100,
            duration_ms: 5000,
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("JobFinished"));
        assert!(json.contains("5000"));
    }

    #[test]
    fn test_progress_event_message_serialization() {
        let event = ProgressEvent::Message {
            level: "info".to_string(),
            message: "Processing started".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("Message"));
        assert!(json.contains("Processing started"));
    }

    #[test]
    fn test_progress_event_status_snapshot_serialization() {
        let event = ProgressEvent::StatusSnapshot {
            is_busy: true,
            job_name: Some("ingestion".to_string()),
            processed_documents: 50,
            total_documents: 100,
            current_batch: 3,
            total_batches: 10,
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("StatusSnapshot"));
        assert!(json.contains("ingestion"));
    }

    #[test]
    fn test_progress_event_heartbeat_serialization() {
        let event = ProgressEvent::Heartbeat {
            timestamp: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("Heartbeat"));
    }

    #[test]
    fn test_progress_event_connected_serialization() {
        let event = ProgressEvent::Connected {
            message: "Connected successfully".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("Connected"));
    }

    #[test]
    fn test_progress_event_cancellation_serialization() {
        let event = ProgressEvent::CancellationRequested;

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("CancellationRequested"));
    }

    #[tokio::test]
    async fn test_progress_broadcaster_creation() {
        let broadcaster = ProgressBroadcaster::new(100);
        let mut rx = broadcaster.subscribe();

        broadcaster.job_started("test-job", 10, 2);

        let event = rx.recv().await.unwrap();
        match event {
            ProgressEvent::JobStarted {
                job_name,
                total_documents,
                total_batches,
            } => {
                assert_eq!(job_name, "test-job");
                assert_eq!(total_documents, 10);
                assert_eq!(total_batches, 2);
            }
            _ => panic!("Unexpected event type"),
        }
    }

    #[tokio::test]
    async fn test_progress_broadcaster_multiple_subscribers() {
        let broadcaster = ProgressBroadcaster::new(100);
        let mut rx1 = broadcaster.subscribe();
        let mut rx2 = broadcaster.subscribe();

        broadcaster.document_progress("doc-1", 5, 1, 10);

        let event1 = rx1.recv().await.unwrap();
        let event2 = rx2.recv().await.unwrap();

        match (&event1, &event2) {
            (
                ProgressEvent::DocumentProgress {
                    document_id: id1, ..
                },
                ProgressEvent::DocumentProgress {
                    document_id: id2, ..
                },
            ) => {
                assert_eq!(id1, "doc-1");
                assert_eq!(id2, "doc-1");
            }
            _ => panic!("Unexpected event types"),
        }
    }

    /// Test PdfPageProgress event serialization.
    ///
    /// @implements OODA-06: Verify PdfPageProgress event structure
    #[test]
    fn test_progress_event_pdf_page_progress_serialization() {
        let event = ProgressEvent::PdfPageProgress {
            pdf_id: "pdf-123".to_string(),
            task_id: "task-456".to_string(),
            page_num: 5,
            total_pages: 30,
            phase: "extraction".to_string(),
            markdown_len: 1024,
            success: true,
            error: None,
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("PdfPageProgress"));
        assert!(json.contains("pdf-123"));
        assert!(json.contains("\"page_num\":5"));
        assert!(json.contains("\"total_pages\":30"));
        assert!(json.contains("\"phase\":\"extraction\""));
        assert!(json.contains("\"success\":true"));
    }

    /// Test PdfPageProgress event with error.
    #[test]
    fn test_progress_event_pdf_page_progress_with_error() {
        let event = ProgressEvent::PdfPageProgress {
            pdf_id: "pdf-123".to_string(),
            task_id: "task-456".to_string(),
            page_num: 3,
            total_pages: 10,
            phase: "extraction".to_string(),
            markdown_len: 0,
            success: false,
            error: Some("Failed to decode font".to_string()),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("PdfPageProgress"));
        assert!(json.contains("\"success\":false"));
        assert!(json.contains("Failed to decode font"));
    }

    // -------------------------------------------------------------------------
    // Tests for IngestionCompletedEvent (Option B parallel raw-String channel)
    // -------------------------------------------------------------------------

    /// Serializes an IngestionCompletedEvent WITH a snapshot block and asserts
    /// the flat snake_case wire shape (no top-level "data" envelope, no PascalCase).
    #[test]
    fn test_ingestion_completed_with_snapshot_serialization() {
        let event = IngestionCompletedEvent::new(
            "rebuild_kg_abc123".to_string(),
            "workspace-uuid-1".to_string(),
            "2024-06-11T12:00:00Z".to_string(),
            42_000,
            IngestionSummary {
                chunks: 50,
                entities: 120,
                relationships: 80,
                total_cost_usd: 0.0,
            },
            Some(IngestionSnapshot {
                uri: "/snapshots/my-ns".to_string(),
                created_at: "2024-06-11T12:00:05Z".to_string(),
                counts: IngestionCounts {
                    documents: 10,
                    chunks: 50,
                    entities: 120,
                    relationships: 80,
                    vectors: 170,
                    bm25: 50,
                },
            }),
        );

        let json = serde_json::to_string(&event).unwrap();

        // Flat type discriminator must be present at top level.
        assert!(json.contains("\"type\":\"ingestion_completed\""), "missing flat type field: {json}");
        assert!(json.contains("\"track_id\""), "missing track_id: {json}");
        assert!(json.contains("\"total_duration_ms\""), "missing total_duration_ms: {json}");
        assert!(json.contains("\"summary\""), "missing summary: {json}");
        assert!(json.contains("\"snapshot\""), "missing snapshot block: {json}");
        assert!(json.contains("\"counts\""), "missing counts in snapshot: {json}");

        // Must NOT have a top-level "data" envelope (PascalCase ProgressEvent pattern).
        assert!(!json.contains("\"data\":"), "unexpected PascalCase data envelope: {json}");

        // Must NOT contain secret/token/key/password (T-24-02-01 threat check).
        assert!(!json.to_lowercase().contains("secret"), "payload contains 'secret': {json}");
        assert!(!json.to_lowercase().contains("password"), "payload contains 'password': {json}");
    }

    /// Serializes an IngestionCompletedEvent WITHOUT a snapshot block (no-snapshot
    /// completion, scenario 5 / scratch workspace).  The "snapshot" key must be
    /// absent from the JSON thanks to `skip_serializing_if = "Option::is_none"`.
    #[test]
    fn test_ingestion_completed_without_snapshot_serialization() {
        let event = IngestionCompletedEvent::new(
            "rebuild_kg_xyz".to_string(),
            "workspace-uuid-2".to_string(),
            "2024-06-11T13:00:00Z".to_string(),
            5_000,
            IngestionSummary {
                chunks: 0,
                entities: 0,
                relationships: 0,
                total_cost_usd: 0.0,
            },
            None, // no snapshot
        );

        let json = serde_json::to_string(&event).unwrap();

        // The type field must still be present.
        assert!(json.contains("\"type\":\"ingestion_completed\""), "missing type: {json}");

        // The "snapshot" key must be entirely absent.
        assert!(!json.contains("\"snapshot\""), "snapshot should be absent when None: {json}");

        // No data envelope.
        assert!(!json.contains("\"data\":"), "unexpected data envelope: {json}");
    }

    /// End-to-end test: subscribe_raw → broadcast_ingestion_completed → recv.
    /// Proves the parallel raw-String channel delivers the flat payload verbatim.
    #[tokio::test]
    async fn test_broadcast_ingestion_completed_via_raw_channel() {
        let broadcaster = ProgressBroadcaster::new(100);
        let mut raw_rx = broadcaster.subscribe_raw();

        broadcaster.broadcast_ingestion_completed(IngestionCompletedEvent::new(
            "rebuild_kg_test".to_string(),
            "ws-id".to_string(),
            "2024-06-11T14:00:00Z".to_string(),
            1_000,
            IngestionSummary {
                chunks: 1,
                entities: 2,
                relationships: 1,
                total_cost_usd: 0.0,
            },
            None,
        ));

        let received = raw_rx.recv().await.expect("raw channel must deliver the event");
        assert!(
            received.contains("ingestion_completed"),
            "raw channel payload must contain 'ingestion_completed': {received}"
        );
    }

    /// Regression test: the existing typed ProgressEvent::JobStarted still
    /// serializes with PascalCase type + "data" envelope (no regression to
    /// the typed channel).
    #[test]
    fn test_progress_event_job_started_still_pascal_case() {
        let event = ProgressEvent::JobStarted {
            job_name: "regression-check".to_string(),
            total_documents: 5,
            total_batches: 1,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"JobStarted\""), "typed channel must keep PascalCase type: {json}");
        assert!(json.contains("\"data\""), "typed channel must keep 'data' envelope: {json}");
    }
}
