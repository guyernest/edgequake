//! Raw document S3 storage client.
//!
//! Implements the "S3-as-record" design for Phase 143: the uploaded S3 object is the
//! durable record of a raw document upload. No DynamoDB row, no confirm endpoint.
//!
//! # Key layout
//!
//! `{tenant}/{workspace}/{namespace}/{sha256}/{filename}`
//!
//! - `tenant` — server-injected X-Tenant-ID UUID (not client-supplied)
//! - `workspace` — server-injected X-Workspace-ID or "default"
//! - `namespace` — client-supplied (validated, no traversal)
//! - `sha256` — content-addressed document_id (advisory for dedup, D-03)
//! - `filename` — client-supplied (validated, no traversal)
//!
//! # RawDocsReader seam (Phase 33-12)
//!
//! The [`RawDocsReader`] trait is the MANDATORY injectable seam for the schema sampler.
//! Production code reads through [`RawDocsStorageReader`] (delegates to [`RawDocsStorage`]).
//! Tests inject [`InMemoryRawDocsReader`] so the sampler runs without a live S3 bucket.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;

use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::presigning::PresigningConfig;

use crate::error::AwsStorageError;

// ============================================================================
// RawDocsReader seam (Phase 33-12, Codex HIGH-3)
// ============================================================================

/// A single raw document object returned from `list_raw_docs`.
///
/// Shared by both the real [`RawDocsStorage`] and the injectable seam.
#[derive(Debug, Clone)]
pub struct RawObject {
    /// Full S3 object key.
    pub key: String,
    /// Object size in bytes.
    pub size: i64,
    /// Last modified timestamp (ISO-8601 string), if available.
    pub last_modified: Option<String>,
}

/// Injectable seam for raw-document read access (listing + retrieval).
///
/// This trait is the MANDATORY seam that separates the schema sampler from live S3.
/// The production path uses [`RawDocsStorageReader`] (delegates to [`RawDocsStorage`]).
/// Tests inject [`InMemoryRawDocsReader`] — no real bucket, no `#[ignore]`.
///
/// The trait mirrors the concrete [`RawDocsStorage`] methods exactly so production
/// callers are byte-for-byte identical to the test path.
#[async_trait]
pub trait RawDocsReader: Send + Sync {
    /// List all raw documents under `prefix` (paginated internally, returns ALL).
    ///
    /// `prefix` should end with a trailing slash: `"{tenant}/{workspace}/{namespace}/"`.
    ///
    /// # Errors
    ///
    /// Returns `AwsStorageError` on any underlying error (S3 / in-memory).
    async fn list_raw_docs(&self, prefix: &str) -> Result<Vec<RawObject>, AwsStorageError>;

    /// Retrieve the bytes of a raw document at `key`.
    ///
    /// # Errors
    ///
    /// Returns `AwsStorageError` when the key is unknown or an I/O error occurs.
    async fn get_object(&self, key: &str) -> Result<Vec<u8>, AwsStorageError>;
}

/// Production [`RawDocsReader`] impl — delegates verbatim to [`RawDocsStorage`].
///
/// The live `new_dynamodb` path is byte-for-byte unchanged: this wrapper just
/// exposes the same methods behind the trait so the sampler can be injected in tests.
pub struct RawDocsStorageReader<'a> {
    inner: &'a RawDocsStorage,
}

impl<'a> RawDocsStorageReader<'a> {
    /// Wrap a [`RawDocsStorage`] reference.
    pub fn new(inner: &'a RawDocsStorage) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl<'a> RawDocsReader for RawDocsStorageReader<'a> {
    async fn list_raw_docs(&self, prefix: &str) -> Result<Vec<RawObject>, AwsStorageError> {
        self.inner.list_raw_docs(prefix).await
    }

    async fn get_object(&self, key: &str) -> Result<Vec<u8>, AwsStorageError> {
        self.inner.get_object(key).await
    }
}

/// In-memory fake [`RawDocsReader`] for tests.
///
/// Holds a `Vec<(key, bytes)>`.
/// - `list_raw_docs(prefix)` returns only entries whose key STARTS WITH `prefix`.
/// - `get_object(key)` returns the seeded bytes, or an error for unknown keys.
/// No real S3 calls, no `#[ignore]` — runs in CI.
pub struct InMemoryRawDocsReader {
    objects: Vec<(String, Vec<u8>)>,
}

impl InMemoryRawDocsReader {
    /// Construct an empty fake reader.
    pub fn new() -> Self {
        Self { objects: Vec::new() }
    }

    /// Seed an object with the given key and byte content.
    pub fn seed(&mut self, key: impl Into<String>, bytes: impl Into<Vec<u8>>) {
        self.objects.push((key.into(), bytes.into()));
    }

    /// Convenience builder: seed from an iterator of `(key, bytes)` pairs.
    pub fn from_objects<K, V>(pairs: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String>,
        V: Into<Vec<u8>>,
    {
        let mut reader = Self::new();
        for (k, v) in pairs {
            reader.seed(k, v);
        }
        reader
    }
}

impl Default for InMemoryRawDocsReader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RawDocsReader for InMemoryRawDocsReader {
    async fn list_raw_docs(&self, prefix: &str) -> Result<Vec<RawObject>, AwsStorageError> {
        let objects = self
            .objects
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(key, bytes)| RawObject {
                key: key.clone(),
                size: bytes.len() as i64,
                last_modified: None,
            })
            .collect();
        Ok(objects)
    }

    async fn get_object(&self, key: &str) -> Result<Vec<u8>, AwsStorageError> {
        self.objects
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, bytes)| bytes.clone())
            .ok_or_else(|| AwsStorageError::S3Error(format!("InMemoryRawDocsReader: key not found: {key}")))
    }
}

// ============================================================================
// RawDocsStorage (S3 client)
// ============================================================================

/// S3 client for raw document storage (Phase 143 "S3-as-record").
///
/// Constructed once at `AppState` initialization from the `RAW_DOCS_BUCKET` env var.
/// An empty bucket name is acceptable for unit-test / in-memory paths (no S3 calls will
/// be made in that case).
pub struct RawDocsStorage {
    s3_client: S3Client,
    bucket: String,
}

impl RawDocsStorage {
    /// Create a new `RawDocsStorage`.
    ///
    /// Loads AWS configuration from the environment (region, credentials).
    /// `bucket` is the raw-docs S3 bucket name (read from `RAW_DOCS_BUCKET` env var by the
    /// caller). An empty bucket string is accepted without panic (unit-test / in-memory path).
    pub async fn new(bucket: String) -> Self {
        let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let s3_client = S3Client::new(&aws_config);
        Self { s3_client, bucket }
    }

    /// Validate a single S3 key segment (namespace, filename, workspace).
    ///
    /// Rejects: empty strings, "." and "..", values containing '/', '\\', NUL, or any
    /// ASCII control character.
    ///
    /// # Errors
    ///
    /// Returns `AwsStorageError::S3Error` with a descriptive message when validation fails.
    pub fn validate_segment(label: &str, value: &str) -> Result<(), AwsStorageError> {
        if value.is_empty() {
            return Err(AwsStorageError::S3Error(format!(
                "invalid {label} key segment: must not be empty"
            )));
        }
        if value == "." || value == ".." {
            return Err(AwsStorageError::S3Error(format!(
                "invalid {label} key segment: '{}' is a path traversal value",
                value
            )));
        }
        for ch in value.chars() {
            if ch == '/' || ch == '\\' || ch == '\0' || (ch.is_ascii() && ch.is_ascii_control()) {
                return Err(AwsStorageError::S3Error(format!(
                    "invalid {label} key segment: contains forbidden character '{}'",
                    ch.escape_debug()
                )));
            }
        }
        Ok(())
    }

    /// Build the content-addressed S3 key for a raw document.
    ///
    /// Layout: `{tenant}/{workspace}/{namespace}/{sha256}/{filename}` (D-09).
    /// document_id == sha256 (content-addressed, D-03).
    ///
    /// Validates `namespace`, `filename`, and `workspace` as client-controlled segments
    /// (T-143-01, T-143-17). `tenant` is server-injected and validated for defense-in-depth.
    ///
    /// # Errors
    ///
    /// Returns `AwsStorageError::S3Error` when any segment fails validation.
    pub fn build_key(
        tenant: &str,
        workspace: &str,
        namespace: &str,
        sha256: &str,
        filename: &str,
    ) -> Result<String, AwsStorageError> {
        // Validate all segments (tenant is server-injected but validate for defense-in-depth)
        Self::validate_segment("tenant", tenant)?;
        Self::validate_segment("workspace", workspace)?;
        Self::validate_segment("namespace", namespace)?;
        Self::validate_segment("sha256", sha256)?;
        Self::validate_segment("filename", filename)?;

        Ok(format!("{tenant}/{workspace}/{namespace}/{sha256}/{filename}"))
    }

    /// Mint a presigned S3 PUT URL for a raw document upload.
    ///
    /// Signs `x-amz-meta-filename`, `x-amz-meta-sha256`, and `x-amz-meta-namespace`
    /// as metadata headers. If `content_type` is provided, it is also signed (so the
    /// browser must echo it byte-for-byte).
    ///
    /// Returns `(upload_url, upload_headers)` where `upload_headers` is the exact set of
    /// request headers the browser must include in the PUT request (review concern #1).
    ///
    /// URL expires in 15 minutes (T-143-03).
    ///
    /// # Errors
    ///
    /// Returns `AwsStorageError::S3Error` on any SDK error.
    pub async fn mint_presigned_put(
        &self,
        key: &str,
        filename: &str,
        sha256: &str,
        namespace: &str,
        content_type: Option<&str>,
    ) -> Result<(String, BTreeMap<String, String>), AwsStorageError> {
        let presign_config = PresigningConfig::expires_in(Duration::from_secs(15 * 60))
            .map_err(|e| AwsStorageError::S3Error(format!("Failed to build presign config: {e}")))?;

        let mut builder = self
            .s3_client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .metadata("filename", filename)
            .metadata("sha256", sha256)
            .metadata("namespace", namespace);

        if let Some(ct) = content_type {
            builder = builder.content_type(ct);
        }

        let presigned = builder
            .presigned(presign_config)
            .await
            .map_err(|e| AwsStorageError::S3Error(format!("Failed to mint presigned PUT: {e}")))?;

        let upload_url = presigned.uri().to_string();

        // Build the header map the browser MUST echo on the PUT (signed metadata becomes
        // x-amz-meta-{key} headers; S3 returns 403 SignatureDoesNotMatch if omitted).
        let mut headers = BTreeMap::new();
        headers.insert("x-amz-meta-filename".to_string(), filename.to_string());
        headers.insert("x-amz-meta-sha256".to_string(), sha256.to_string());
        headers.insert("x-amz-meta-namespace".to_string(), namespace.to_string());
        if let Some(ct) = content_type {
            headers.insert("content-type".to_string(), ct.to_string());
        }

        Ok((upload_url, headers))
    }

    /// Check whether a raw document already exists at `key` (content-addressed dedup, D-03).
    ///
    /// Returns `Ok(true)` when the object exists, `Ok(false)` when it is genuinely absent
    /// (HTTP 404 / `NoSuchKey`), and `Err(AwsStorageError::S3Error(...))` for any other
    /// S3 error.
    ///
    /// Uses `HeadObject` — no bytes are transferred.
    pub async fn head_exists(&self, key: &str) -> Result<bool, AwsStorageError> {
        use aws_sdk_s3::error::SdkError;

        match self
            .s3_client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(SdkError::ServiceError(err)) => {
                // HeadObjectError does not expose a named NotFound variant in all SDK versions;
                // check by HTTP status code (404) to distinguish genuine absence from other errors.
                if err.raw().status().as_u16() == 404 {
                    Ok(false)
                } else {
                    Err(AwsStorageError::S3Error(format!(
                        "HeadObject failed: {}",
                        err.err()
                    )))
                }
            }
            Err(e) => Err(AwsStorageError::S3Error(format!("HeadObject failed: {e}"))),
        }
    }

    /// Retrieve the bytes of a raw document stored at `key`.
    ///
    /// Returns the full object body as a `Vec<u8>`, or `AwsStorageError` when the
    /// object is missing or any S3 error occurs.
    ///
    /// Used by the AI schema-draft endpoint to sample workspace docs, and by any
    /// code-path that must read content back from the raw-docs bucket in-Lambda.
    ///
    /// # Errors
    ///
    /// Returns `AwsStorageError::S3Error` on any SDK or body-read error.
    pub async fn get_object(&self, key: &str) -> Result<Vec<u8>, AwsStorageError> {
        let response = self
            .s3_client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                AwsStorageError::S3Error(format!("get_object '{}': {}", key, e))
            })?;

        let body = response
            .body
            .collect()
            .await
            .map_err(|e| {
                AwsStorageError::S3Error(format!("read body '{}': {}", key, e))
            })?;

        Ok(body.into_bytes().to_vec())
    }

    /// Store `bytes` in S3 at `key` with the given `content_type`.
    ///
    /// Used by legacy upload handlers (`upload_file`, `upload_document`) to land file
    /// bytes in the raw-docs bucket before writing document metadata. The caller should
    /// use `build_key` to derive `key` so the key layout is consistent with the
    /// presigned-upload path.
    ///
    /// # Errors
    ///
    /// Returns `AwsStorageError::S3Error` on any SDK error.
    pub async fn put_bytes(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), AwsStorageError> {
        use aws_sdk_s3::primitives::ByteStream;

        self.s3_client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(bytes))
            .content_type(content_type)
            .send()
            .await
            .map_err(|e| {
                AwsStorageError::S3Error(format!("put_bytes '{}': {}", key, e))
            })?;

        Ok(())
    }

    /// List raw documents under `prefix`, following continuation tokens to return ALL
    /// matching objects (review concern #6 — do not silently cap at 1,000 objects).
    ///
    /// `prefix` should be `{tenant}/{workspace}/{namespace}/` (with trailing slash).
    ///
    /// # Errors
    ///
    /// Returns `AwsStorageError::S3Error` on any SDK error.
    pub async fn list_raw_docs(&self, prefix: &str) -> Result<Vec<RawObject>, AwsStorageError> {
        let mut objects: Vec<RawObject> = Vec::new();
        let mut token: Option<String> = None;

        loop {
            let response = self
                .s3_client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix)
                .set_continuation_token(token.clone())
                .send()
                .await
                .map_err(|e| {
                    AwsStorageError::S3Error(format!(
                        "Failed to list raw docs under prefix '{}': {}",
                        prefix, e
                    ))
                })?;

            for obj in response.contents() {
                let key = obj.key().unwrap_or("").to_string();
                let size = obj.size().unwrap_or(0);
                let last_modified = obj
                    .last_modified()
                    .map(|dt| dt.fmt(aws_smithy_types::date_time::Format::DateTime))
                    .and_then(|r| r.ok());

                objects.push(RawObject {
                    key,
                    size,
                    last_modified,
                });
            }

            if response.is_truncated() == Some(true) {
                token = response.next_continuation_token().map(str::to_string);
            } else {
                break;
            }
        }

        Ok(objects)
    }
}

// ============================================================================
// Unit tests (pure-logic; no S3 calls)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------------
    // build_key layout
    // ---------------------------------------------------------------------------

    #[test]
    fn build_key_layout() {
        let key = RawDocsStorage::build_key(
            "tenant-uuid",
            "ws-1",
            "my-namespace",
            "abc123def456abc123def456abc123def456abc123def456abc123def456abc1",
            "report.pdf",
        )
        .unwrap();
        assert_eq!(
            key,
            "tenant-uuid/ws-1/my-namespace/abc123def456abc123def456abc123def456abc123def456abc123def456abc1/report.pdf"
        );
        // document_id == sha256 segment (content-addressed, D-03)
        let segments: Vec<&str> = key.split('/').collect();
        assert_eq!(
            segments[3],
            "abc123def456abc123def456abc123def456abc123def456abc123def456abc1"
        );
    }

    // ---------------------------------------------------------------------------
    // validate_segment rejections
    // ---------------------------------------------------------------------------

    #[test]
    fn validate_segment_rejects_empty() {
        assert!(RawDocsStorage::validate_segment("namespace", "").is_err());
        assert!(RawDocsStorage::validate_segment("filename", "").is_err());
    }

    #[test]
    fn validate_segment_rejects_dot_dot() {
        assert!(RawDocsStorage::validate_segment("namespace", "..").is_err());
        assert!(RawDocsStorage::validate_segment("filename", "..").is_err());
    }

    #[test]
    fn validate_segment_rejects_single_dot() {
        assert!(RawDocsStorage::validate_segment("namespace", ".").is_err());
    }

    #[test]
    fn validate_segment_rejects_slash() {
        assert!(RawDocsStorage::validate_segment("namespace", "a/b").is_err());
        assert!(RawDocsStorage::validate_segment("filename", "a/b").is_err());
    }

    #[test]
    fn validate_segment_rejects_backslash() {
        assert!(RawDocsStorage::validate_segment("namespace", "a\\b").is_err());
        assert!(RawDocsStorage::validate_segment("filename", "a\\b").is_err());
    }

    #[test]
    fn validate_segment_rejects_nul() {
        assert!(RawDocsStorage::validate_segment("namespace", "a\0b").is_err());
    }

    #[test]
    fn validate_segment_rejects_control_char() {
        assert!(RawDocsStorage::validate_segment("namespace", "a\x01b").is_err());
        assert!(RawDocsStorage::validate_segment("filename", "a\tb").is_err()); // tab is ASCII control
    }

    #[test]
    fn validate_segment_accepts_valid() {
        assert!(RawDocsStorage::validate_segment("namespace", "my-namespace").is_ok());
        assert!(RawDocsStorage::validate_segment("filename", "report.pdf").is_ok());
        assert!(RawDocsStorage::validate_segment("workspace", "default").is_ok());
        assert!(RawDocsStorage::validate_segment("sha256", "abc123").is_ok());
    }

    #[test]
    fn build_key_rejects_traversal_in_namespace() {
        assert!(RawDocsStorage::build_key("t", "w", "..", "sha", "file.txt").is_err());
        assert!(RawDocsStorage::build_key("t", "w", "a/b", "sha", "file.txt").is_err());
    }

    #[test]
    fn build_key_rejects_traversal_in_filename() {
        assert!(RawDocsStorage::build_key("t", "w", "ns", "sha", "..").is_err());
        assert!(RawDocsStorage::build_key("t", "w", "ns", "sha", "a/b.txt").is_err());
    }

    #[test]
    fn build_key_rejects_traversal_in_workspace() {
        assert!(RawDocsStorage::build_key("t", "..", "ns", "sha", "file.txt").is_err());
        assert!(RawDocsStorage::build_key("t", "a/b", "ns", "sha", "file.txt").is_err());
    }

    // ---------------------------------------------------------------------------
    // RawDocsReader seam tests (Phase 33-12, Codex HIGH-3)
    // These tests prove the in-memory fake works correctly and that the seam
    // is injectable. No real S3 calls. No #[ignore]. Runs in CI.
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn raw_docs_reader_list_filters_by_prefix() {
        // Seed two keys under the target prefix and one under a different prefix.
        // list_raw_docs(prefix) must return ONLY the two matching keys.
        let reader = InMemoryRawDocsReader::from_objects([
            ("tenant1/ws1/ns1/sha-a/doc-a.md", b"content A" as &[u8]),
            ("tenant1/ws1/ns1/sha-b/doc-b.md", b"content B" as &[u8]),
            ("tenant2/ws2/ns2/sha-c/doc-c.md", b"content C" as &[u8]), // different prefix
        ]);

        let prefix = "tenant1/ws1/ns1/";
        let objects = reader.list_raw_docs(prefix).await.unwrap();

        assert_eq!(objects.len(), 2, "Only 2 objects match the prefix");
        let keys: Vec<&str> = objects.iter().map(|o| o.key.as_str()).collect();
        assert!(keys.contains(&"tenant1/ws1/ns1/sha-a/doc-a.md"));
        assert!(keys.contains(&"tenant1/ws1/ns1/sha-b/doc-b.md"));
        assert!(!keys.contains(&"tenant2/ws2/ns2/sha-c/doc-c.md"));
    }

    #[tokio::test]
    async fn raw_docs_reader_get_object_returns_seeded_bytes() {
        // get_object must return exactly the seeded bytes.
        let reader = InMemoryRawDocsReader::from_objects([
            ("tenant1/ws1/ns1/sha-a/doc.md", b"hello seam" as &[u8]),
        ]);

        let bytes = reader.get_object("tenant1/ws1/ns1/sha-a/doc.md").await.unwrap();
        assert_eq!(bytes, b"hello seam");
    }

    #[tokio::test]
    async fn raw_docs_reader_get_object_errors_for_unknown_key() {
        let reader = InMemoryRawDocsReader::new();
        let result = reader.get_object("nonexistent/key").await;
        assert!(result.is_err(), "Unknown key must return Err");
    }

    #[tokio::test]
    async fn raw_docs_reader_size_matches_content_length() {
        let reader = InMemoryRawDocsReader::from_objects([
            ("t/w/ns/sha/file.md", b"twelve bytes" as &[u8]),
        ]);
        let objects = reader.list_raw_docs("t/w/ns/").await.unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].size, 12, "size must equal byte length");
    }

    #[tokio::test]
    async fn raw_docs_reader_empty_prefix_returns_all() {
        // With an empty prefix every seeded key matches.
        let reader = InMemoryRawDocsReader::from_objects([
            ("a/b", b"1" as &[u8]),
            ("c/d", b"2" as &[u8]),
        ]);
        let objects = reader.list_raw_docs("").await.unwrap();
        assert_eq!(objects.len(), 2);
    }
}
