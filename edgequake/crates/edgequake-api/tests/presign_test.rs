//! Pure-logic unit tests for the Phase 143 presigned-S3 raw-docs upload path.
//!
//! These tests exercise:
//! (a) S3 key layout and document_id == sha256 (content-addressed, D-03)
//! (b) validate_segment rejections (namespace, filename, workspace) (T-143-01, T-143-17)
//! (c) StatusCounts default includes uploaded field (D-07)
//! (d) PresignResponse::duplicate shape (review concern #1)
//!
//! Network tests (presign/head/list) are integration-only and are NOT in this file.

// No tokio runtime needed — all tests are synchronous.

use edgequake_api::handlers::documents_types::{PresignResponse, RawDocSummary, StatusCounts};
use edgequake_storage_aws::RawDocsStorage;

// ============================================================================
// (a) S3 key layout — document_id == sha256 (D-03)
// ============================================================================

#[test]
fn presign_key_layout_is_tenant_workspace_namespace_sha256_filename() {
    let sha256 = "a".repeat(64);
    let key = RawDocsStorage::build_key(
        "tenant-uuid",
        "ws-abc",
        "legal-docs",
        &sha256,
        "contract.pdf",
    )
    .expect("build_key should succeed for valid segments");

    // Exact layout: {tenant}/{workspace}/{namespace}/{sha256}/{filename}
    assert_eq!(
        key,
        format!("tenant-uuid/ws-abc/legal-docs/{}/contract.pdf", sha256)
    );

    // document_id == sha256 segment (4th, 0-indexed)
    let segments: Vec<&str> = key.split('/').collect();
    assert_eq!(segments.len(), 5);
    assert_eq!(segments[0], "tenant-uuid");
    assert_eq!(segments[1], "ws-abc");
    assert_eq!(segments[2], "legal-docs");
    assert_eq!(segments[3], sha256);    // document_id == sha256
    assert_eq!(segments[4], "contract.pdf");
}

// ============================================================================
// (b) validate_segment — namespace rejections
// ============================================================================

#[test]
fn validate_namespace_rejects_empty() {
    assert!(
        RawDocsStorage::validate_segment("namespace", "").is_err(),
        "empty namespace must be rejected"
    );
}

#[test]
fn validate_namespace_rejects_dot_dot() {
    assert!(
        RawDocsStorage::validate_segment("namespace", "..").is_err(),
        "'..' namespace must be rejected"
    );
}

#[test]
fn validate_namespace_rejects_slash() {
    assert!(
        RawDocsStorage::validate_segment("namespace", "a/b").is_err(),
        "namespace with '/' must be rejected"
    );
}

#[test]
fn validate_namespace_rejects_backslash() {
    assert!(
        RawDocsStorage::validate_segment("namespace", "a\\b").is_err(),
        "namespace with '\\\\' must be rejected"
    );
}

#[test]
fn validate_namespace_rejects_control_char() {
    assert!(
        RawDocsStorage::validate_segment("namespace", "a\x01b").is_err(),
        "namespace with ASCII control char must be rejected"
    );
}

// ============================================================================
// (b) validate_segment — filename rejections
// ============================================================================

#[test]
fn validate_filename_rejects_empty() {
    assert!(
        RawDocsStorage::validate_segment("filename", "").is_err(),
        "empty filename must be rejected"
    );
}

#[test]
fn validate_filename_rejects_dot_dot() {
    assert!(
        RawDocsStorage::validate_segment("filename", "..").is_err(),
        "'..' filename must be rejected"
    );
}

#[test]
fn validate_filename_rejects_slash() {
    assert!(
        RawDocsStorage::validate_segment("filename", "a/b.txt").is_err(),
        "filename with '/' must be rejected"
    );
}

#[test]
fn validate_filename_rejects_nul() {
    assert!(
        RawDocsStorage::validate_segment("filename", "file\0name").is_err(),
        "filename with NUL must be rejected"
    );
}

#[test]
fn validate_filename_accepts_valid() {
    assert!(
        RawDocsStorage::validate_segment("filename", "report-2026.pdf").is_ok(),
        "normal filename should be accepted"
    );
}

// ============================================================================
// (b) validate_segment — workspace rejections (T-143-17 client-controlled segment)
// ============================================================================

#[test]
fn validate_workspace_rejects_traversal() {
    assert!(
        RawDocsStorage::validate_segment("workspace", "..").is_err(),
        "'..' workspace must be rejected"
    );
    assert!(
        RawDocsStorage::validate_segment("workspace", "a/b").is_err(),
        "workspace with '/' must be rejected"
    );
}

#[test]
fn validate_workspace_accepts_default() {
    assert!(
        RawDocsStorage::validate_segment("workspace", "default").is_ok(),
        "\"default\" workspace must be accepted"
    );
}

// ============================================================================
// (c) StatusCounts default includes uploaded field (D-07)
// ============================================================================

#[test]
fn status_counts_default_has_uploaded_field() {
    let counts = StatusCounts::default();
    assert_eq!(counts.uploaded, 0, "Default StatusCounts.uploaded must be 0");
    // Verify the other fields are also 0
    assert_eq!(counts.pending, 0);
    assert_eq!(counts.processing, 0);
    assert_eq!(counts.completed, 0);
    assert_eq!(counts.failed, 0);
    assert_eq!(counts.cancelled, 0);
}

#[test]
fn status_counts_serialize_includes_uploaded() {
    let counts = StatusCounts {
        pending: 1,
        processing: 0,
        completed: 5,
        partial_failure: 0,
        failed: 0,
        cancelled: 0,
        uploaded: 3,
    };
    let json = serde_json::to_string(&counts).expect("serialize");
    assert!(json.contains("\"uploaded\":3"), "Serialized JSON must include uploaded field");
}

// ============================================================================
// (d) PresignResponse::duplicate — no URL, empty headers, is_duplicate:true
// ============================================================================

#[test]
fn presign_response_duplicate_has_no_url() {
    let sha256 = "b".repeat(64);
    let resp = PresignResponse::duplicate(sha256.clone());

    assert_eq!(resp.document_id, sha256, "document_id must be the sha256");
    assert!(resp.upload_url.is_none(), "upload_url must be None for duplicates");
    assert!(resp.upload_headers.is_empty(), "upload_headers must be empty for duplicates");
    assert!(resp.s3_key.is_none(), "s3_key must be None for duplicates");
    assert_eq!(resp.status, "uploaded", "status must be 'uploaded' for duplicates");
    assert!(resp.is_duplicate, "is_duplicate must be true");
}

#[test]
fn presign_response_duplicate_serializes_correctly() {
    let resp = PresignResponse::duplicate("deadbeef".repeat(8));
    let json = serde_json::to_string(&resp).expect("serialize");

    assert!(json.contains("\"is_duplicate\":true"));
    assert!(json.contains("\"upload_url\":null"));
    assert!(json.contains("\"upload_headers\":{}"));
    assert!(json.contains("\"status\":\"uploaded\""));
}

// ============================================================================
// (e) RawDocSummary shape
// ============================================================================

#[test]
fn raw_doc_summary_status_is_uploaded() {
    let summary = RawDocSummary {
        document_id: "sha".to_string(),
        file_name: "file.txt".to_string(),
        file_size: 1024,
        content_hash: Some("sha".to_string()),
        namespace: "legal".to_string(),
        uploaded_at: Some("2026-06-19T00:00:00Z".to_string()),
        status: "uploaded".to_string(),
    };
    assert_eq!(summary.status, "uploaded");
    let json = serde_json::to_string(&summary).expect("serialize");
    assert!(json.contains("\"status\":\"uploaded\""));
}
