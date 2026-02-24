//! Upfront validation for batch pipeline preconditions.
//!
//! Validates all preconditions before pipeline processing begins. Collects
//! ALL errors and reports them together (not fail-fast on first error) so
//! the user can fix everything in one pass.
//!
//! Note: Namespace existence and schema approval are already validated by
//! `NamespaceConfigResolver::resolve()` which runs before this validation.
//! This module checks the remaining preconditions that the resolver does not cover.

use crate::config::BatchConfig;
use crate::directory_scanner::DirectoryScanner;
use tracing::debug;

/// Validate all preconditions before pipeline execution.
///
/// Collects all errors into a `Vec<String>` and reports them together.
/// Returns `Ok(())` if all checks pass, or `Err` with all validation
/// errors formatted as a numbered list.
///
/// # Checks
///
/// 1. Data path exists
/// 2. Data path has supported files (if directory) or valid extension (if file)
/// 3. API key is non-empty
/// 4. Neptune endpoint is configured
/// 5. Vector bucket is configured
/// 6. S3 bucket for Neptune bulk load is configured
/// 7. Neptune role ARN is configured
pub async fn validate_upfront(config: &BatchConfig, api_key: &str) -> anyhow::Result<()> {
    let mut errors: Vec<String> = Vec::new();

    // 1. Data path exists
    if !config.data_path.exists() {
        errors.push(format!(
            "Data path does not exist: {}",
            config.data_path.display()
        ));
    } else if config.data_path.is_dir() {
        // 2a. Directory: check for supported files
        let scanner = DirectoryScanner::new(&config.data_path)
            .recursive(!config.no_recurse)
            .include_patterns(&config.include_patterns)
            .exclude_patterns(&config.exclude_patterns);

        match scanner.scan() {
            Ok(scan_result) => {
                if scan_result.supported_files().is_empty() {
                    errors.push(format!(
                        "No supported files (.txt, .md) found in directory: {}",
                        config.data_path.display()
                    ));
                } else {
                    debug!(
                        files = scan_result.supported_files().len(),
                        "Data path validated: found supported files"
                    );
                }
            }
            Err(e) => {
                errors.push(format!(
                    "Failed to scan data directory {}: {}",
                    config.data_path.display(),
                    e
                ));
            }
        }
    } else if config.data_path.is_file() {
        // 2b. Single file: check extension
        match config
            .data_path
            .extension()
            .and_then(|e| e.to_str())
        {
            Some("txt") | Some("md") | Some("markdown") | Some("parquet") => {
                debug!(
                    path = %config.data_path.display(),
                    "Data path validated: supported file type"
                );
            }
            Some(ext) => {
                errors.push(format!(
                    "Unsupported file type: .{} (supported: .txt, .md, .parquet)",
                    ext
                ));
            }
            None => {
                errors.push(format!(
                    "Cannot determine file type (no extension): {}",
                    config.data_path.display()
                ));
            }
        }
    }

    // 3. API key is non-empty
    if api_key.trim().is_empty() {
        let key_name = if edgequake_llm::providers::anthropic_batch::is_anthropic_model(
            &config.extraction_model,
        ) {
            "ANTHROPIC_API_KEY"
        } else {
            "OPENAI_API_KEY"
        };
        errors.push(format!("{} is empty", key_name));
    }

    // 4. Neptune endpoint is configured
    if config.neptune_endpoint.is_none() {
        errors.push(
            "Neptune endpoint not configured (set NEPTUNE_ENDPOINT or --neptune-endpoint)"
                .to_string(),
        );
    }

    // 5. Vector bucket is configured
    if config.vector_bucket.is_none() {
        errors.push(
            "Vector bucket not configured (set VECTOR_BUCKET or --vector-bucket)".to_string(),
        );
    }

    // 6. S3 bucket for Neptune bulk load
    if config.s3_bucket.is_none() {
        errors.push(
            "S3 bucket for Neptune bulk load not configured (set S3_BUCKET or --s3-bucket)"
                .to_string(),
        );
    }

    // 7. Neptune role ARN
    if config.neptune_role_arn.is_none() {
        errors.push(
            "Neptune IAM role ARN not configured (set NEPTUNE_ROLE_ARN or --neptune-role-arn)"
                .to_string(),
        );
    }

    // Report all errors at once
    if !errors.is_empty() {
        let count = errors.len();
        let numbered: Vec<String> = errors
            .iter()
            .enumerate()
            .map(|(i, e)| format!("  {}. {}", i + 1, e))
            .collect();
        anyhow::bail!(
            "Validation failed ({} error{}):\n{}",
            count,
            if count == 1 { "" } else { "s" },
            numbered.join("\n")
        );
    }

    debug!("All upfront validation checks passed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_config(data_path: PathBuf) -> BatchConfig {
        BatchConfig {
            job_id: "test-job".to_string(),
            data_path,
            work_dir: PathBuf::from("/tmp/test-work"),
            extraction_model: "gpt-4o-mini".to_string(),
            embedding_model: "text-embedding-3-small".to_string(),
            max_tokens: 4096,
            chunk_size: 512,
            chunk_overlap: 50,
            embedding_batch_size: 100,
            embedding_concurrency: 5,
            state_table: "test-state-table".to_string(),
            registry_table: "test-registry-table".to_string(),
            namespace: "test-ns".to_string(),
            neptune_endpoint: Some("test-endpoint:8182".to_string()),
            s3_bucket: Some("test-bucket".to_string()),
            neptune_role_arn: Some("arn:aws:iam::123456789012:role/test".to_string()),
            vector_bucket: Some("test-vectors".to_string()),
            vector_index: Some("test-index".to_string()),
            limit: 0,
            offset: 0,
            max_retries: 3,
            retry_delay_secs: 30,
            athena_bm25_database: None,
            bm25_s3_bucket: None,
            athena_workgroup: "primary".to_string(),
            athena_output_location: None,
            delimiter: None,
            no_split: false,
            no_recurse: false,
            include_patterns: vec![],
            exclude_patterns: vec![],
            max_failures: 0,
            anthropic_api_key: None,
            embedding_dimension: 1536,
        }
    }

    #[tokio::test]
    async fn test_validate_nonexistent_data_path() {
        let config = test_config(PathBuf::from("/nonexistent/path"));
        let result = validate_upfront(&config, "sk-test-key").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Data path does not exist"));
    }

    #[tokio::test]
    async fn test_validate_empty_api_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "hello world").unwrap();
        let config = test_config(dir.path().to_path_buf());
        let result = validate_upfront(&config, "   ").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("OPENAI_API_KEY is empty"));
    }

    #[tokio::test]
    async fn test_validate_missing_neptune_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "hello world").unwrap();
        let mut config = test_config(dir.path().to_path_buf());
        config.neptune_endpoint = None;
        let result = validate_upfront(&config, "sk-test-key").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Neptune endpoint not configured"));
    }

    #[tokio::test]
    async fn test_validate_collects_multiple_errors() {
        let mut config = test_config(PathBuf::from("/nonexistent/path"));
        config.neptune_endpoint = None;
        config.vector_bucket = None;
        config.s3_bucket = None;
        config.neptune_role_arn = None;
        let result = validate_upfront(&config, "").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        // Should report multiple errors
        assert!(err.contains("5 errors") || err.contains("6 errors"));
        assert!(err.contains("Data path does not exist"));
        assert!(err.contains("OPENAI_API_KEY is empty"));
        assert!(err.contains("Neptune endpoint not configured"));
        assert!(err.contains("Vector bucket not configured"));
        assert!(err.contains("S3 bucket for Neptune bulk load"));
    }

    #[tokio::test]
    async fn test_validate_all_checks_pass() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "hello world").unwrap();
        let config = test_config(dir.path().to_path_buf());
        let result = validate_upfront(&config, "sk-test-key").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_validate_unsupported_file_extension() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.csv");
        std::fs::write(&file, "a,b,c").unwrap();
        let config = test_config(file);
        let result = validate_upfront(&config, "sk-test-key").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unsupported file type: .csv"));
    }

    #[tokio::test]
    async fn test_validate_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path().to_path_buf());
        let result = validate_upfront(&config, "sk-test-key").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No supported files"));
    }
}
