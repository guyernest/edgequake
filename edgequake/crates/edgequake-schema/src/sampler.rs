//! Document sampling from local parquet files or S3 datasets.
//!
//! Supports both local filesystem paths and `s3://bucket/prefix` URIs.
//! Randomly samples a percentage of documents for schema analysis.

use arrow::array::{Array, StringArray};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use rand::prelude::IndexedRandom;
use std::path::Path;
use tracing::{debug, info, warn};

/// A single sampled document.
#[derive(Debug, Clone)]
pub struct SampledDocument {
    /// Document identifier (filename or S3 key).
    pub id: String,
    /// Full text content.
    pub content: String,
    /// Source filename or S3 key.
    pub source: String,
}

/// Result of document sampling.
#[derive(Debug, Clone)]
pub struct SampledDataset {
    /// The sampled documents.
    pub documents: Vec<SampledDocument>,
    /// Total number of documents in the dataset (before sampling).
    pub total_count: usize,
}

/// Sample documents from a dataset at the given location.
///
/// The `location` parameter can be either:
/// - A local filesystem path (file or directory) containing `.parquet` files
/// - An `s3://bucket/prefix` URI pointing to parquet files in S3
///
/// `sample_percentage` controls what fraction of documents to sample (e.g. 10.0 = 10%).
pub async fn sample_documents(
    location: &str,
    sample_percentage: f64,
) -> anyhow::Result<SampledDataset> {
    let all_documents = if location.starts_with("s3://") {
        read_documents_from_s3(location).await?
    } else {
        read_documents_from_local(location)?
    };

    let total_count = all_documents.len();
    if total_count == 0 {
        return Ok(SampledDataset {
            documents: vec![],
            total_count: 0,
        });
    }

    // Calculate sample size: ceil(total * percentage / 100), at least 1
    let sample_size = ((total_count as f64 * sample_percentage / 100.0).ceil() as usize).max(1);
    let sample_size = sample_size.min(total_count);

    info!(
        total = total_count,
        sample_size = sample_size,
        percentage = sample_percentage,
        "Sampling documents"
    );

    // Random sampling
    let mut rng = rand::rng();
    let sampled: Vec<SampledDocument> = all_documents
        .choose_multiple(&mut rng, sample_size)
        .cloned()
        .collect();

    Ok(SampledDataset {
        documents: sampled,
        total_count,
    })
}

/// Strip YAML front matter from markdown content.
///
/// If the content starts with `---\n`, finds the closing `---\n` and returns
/// everything after it. Otherwise returns the input unchanged.
/// This is a minimal inline implementation to avoid a circular dependency
/// on the edgequake-batch crate.
fn strip_yaml_front_matter(input: &str) -> &str {
    let normalized = input.trim_start();
    if !normalized.starts_with("---") {
        return input;
    }

    // Find the closing ---
    let after_first = &normalized[3..];
    let after_first = after_first.trim_start_matches(['\r', '\n']);

    if let Some(end_pos) = after_first.find("\n---") {
        let after_closing = &after_first[end_pos + 4..]; // skip "\n---"
        after_closing.trim_start_matches(['\r', '\n'])
    } else {
        // No closing ---, return input as-is
        input
    }
}

/// Read documents from text/markdown files.
///
/// Handles both a single text/markdown file and a directory containing such files.
/// Uses `walkdir` for directory traversal, skips hidden files.
fn read_documents_from_text_files(location: &str) -> anyhow::Result<Vec<SampledDocument>> {
    let path = Path::new(location);
    let mut documents = Vec::new();

    let files: Vec<std::path::PathBuf> = if path.is_file() {
        vec![path.to_path_buf()]
    } else if path.is_dir() {
        let mut found = Vec::new();
        for entry in walkdir::WalkDir::new(path)
            .into_iter()
            .filter_entry(|e| {
                // Allow root directory, skip hidden entries
                e.depth() == 0
                    || e.file_name()
                        .to_str()
                        .map(|s| !s.starts_with('.'))
                        .unwrap_or(false)
            })
        {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            match entry.path().extension().and_then(|e| e.to_str()) {
                Some("txt") | Some("md") | Some("markdown") => {
                    found.push(entry.path().to_path_buf());
                }
                _ => {}
            }
        }
        found.sort();
        found
    } else {
        anyhow::bail!(
            "Location is not a file or directory: {}",
            path.display()
        );
    };

    if files.is_empty() {
        anyhow::bail!(
            "No text/markdown files found at: {}",
            location
        );
    }

    info!(
        count = files.len(),
        "Found text/markdown files for sampling"
    );

    let base_dir = if path.is_dir() { path } else { path.parent().unwrap_or(Path::new(".")) };

    for file_path in &files {
        match std::fs::read_to_string(file_path) {
            Ok(raw_content) => {
                let relative = file_path
                    .strip_prefix(base_dir)
                    .unwrap_or(file_path)
                    .to_string_lossy()
                    .to_string();

                // For markdown files, strip front matter before sampling
                let content = match file_path.extension().and_then(|e| e.to_str()) {
                    Some("md") | Some("markdown") => {
                        strip_yaml_front_matter(&raw_content).to_string()
                    }
                    _ => raw_content,
                };

                if !content.trim().is_empty() {
                    documents.push(SampledDocument {
                        id: relative.clone(),
                        content,
                        source: relative,
                    });
                }
            }
            Err(e) => {
                warn!(
                    path = %file_path.display(),
                    error = %e,
                    "Skipping file due to read error"
                );
            }
        }
    }

    info!(
        total_documents = documents.len(),
        "Read documents from text/markdown files"
    );

    Ok(documents)
}

/// Read documents from local files (parquet, text, or markdown).
///
/// Handles both single files and directories. For directories, prefers parquet
/// files if any exist (backward compatibility); falls back to text/markdown.
fn read_documents_from_local(location: &str) -> anyhow::Result<Vec<SampledDocument>> {
    let path = Path::new(location);

    if path.is_file() {
        match path.extension().and_then(|e| e.to_str()) {
            Some("parquet") => {
                // Existing parquet reading logic
                let parquet_files = vec![path.to_path_buf()];
                info!(count = parquet_files.len(), "Found parquet files for sampling");
                let mut documents = Vec::new();
                for parquet_path in &parquet_files {
                    let file_docs = read_single_parquet(parquet_path)?;
                    documents.extend(file_docs);
                }
                info!(total_documents = documents.len(), "Read documents from local parquet files");
                return Ok(documents);
            }
            Some("txt") | Some("md") | Some("markdown") => {
                return read_documents_from_text_files(location);
            }
            Some(ext) => {
                anyhow::bail!("Unsupported file type for schema sampling: .{}", ext);
            }
            None => {
                anyhow::bail!("Cannot determine file type: {}", location);
            }
        }
    }

    if path.is_dir() {
        // Check for parquet files first (backward compatibility)
        let parquet_pattern = format!("{}/**/*.parquet", path.display());
        let parquet_files: Vec<_> = glob::glob(&parquet_pattern)?
            .filter_map(|e| e.ok())
            .collect();

        if !parquet_files.is_empty() {
            // Existing parquet directory logic
            info!(
                count = parquet_files.len(),
                "Found parquet files for sampling"
            );
            let mut documents = Vec::new();
            for parquet_path in &parquet_files {
                let file_docs = read_single_parquet(parquet_path)?;
                documents.extend(file_docs);
            }
            info!(
                total_documents = documents.len(),
                "Read documents from local parquet files"
            );
            return Ok(documents);
        }

        // No parquet files -- try text/markdown
        return read_documents_from_text_files(location);
    }

    anyhow::bail!(
        "Location is not a file or directory: {}",
        path.display()
    );
}

/// Read documents from a single parquet file.
///
/// Follows the same CSV-in-parquet format as the batch pipeline's parquet_reader:
/// a `text` column where row 0 is a CSV header and subsequent rows are
/// `filename,text` pairs. Falls back to reading raw text if format differs.
fn read_single_parquet(path: &Path) -> anyhow::Result<Vec<SampledDocument>> {
    let file = std::fs::File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;

    let source = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let mut documents: Vec<SampledDocument> = Vec::new();
    let mut current_filename: Option<String> = None;
    let mut current_lines: Vec<String> = Vec::new();
    let mut total_rows = 0usize;
    let mut is_csv_format = false;

    for batch_result in reader {
        let batch = batch_result?;

        // Try "text" column, then "content" column
        let text_col = batch
            .column_by_name("text")
            .or_else(|| batch.column_by_name("content"));

        let Some(col) = text_col else {
            warn!(
                path = %path.display(),
                "No 'text' or 'content' column found, skipping"
            );
            return Ok(vec![]);
        };

        let text_array = col
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow::anyhow!("Text column is not a string array"))?;

        for i in 0..text_array.len() {
            total_rows += 1;

            if text_array.is_null(i) {
                continue;
            }

            let row_text = text_array.value(i);

            // Detect CSV header on first row
            if total_rows == 1 && row_text.starts_with("filename,") {
                is_csv_format = true;
                debug!("Detected CSV-in-parquet format");
                continue;
            }

            if is_csv_format {
                // CSV-in-parquet format: check for new document start
                if let Some(comma_pos) = row_text.find(',') {
                    let candidate = &row_text[..comma_pos];
                    if candidate.ends_with(".txt")
                        || candidate.ends_with(".pdf")
                        || candidate.ends_with(".doc")
                    {
                        // Flush previous document
                        if let Some(ref filename) = current_filename {
                            if !current_lines.is_empty() {
                                documents.push(SampledDocument {
                                    id: filename.clone(),
                                    content: current_lines.join("\n"),
                                    source: source.clone(),
                                });
                            }
                        }
                        let filename = candidate.trim().to_string();
                        let text = row_text[comma_pos + 1..].to_string();
                        current_filename = Some(filename);
                        current_lines = vec![text];
                        continue;
                    }
                }
                // Continuation line
                current_lines.push(row_text.to_string());
            } else {
                // Simple format: each row is a separate document
                if !row_text.trim().is_empty() {
                    documents.push(SampledDocument {
                        id: format!("{}-row-{}", source, total_rows),
                        content: row_text.to_string(),
                        source: source.clone(),
                    });
                }
            }
        }
    }

    // Flush last CSV document
    if is_csv_format {
        if let Some(ref filename) = current_filename {
            if !current_lines.is_empty() {
                documents.push(SampledDocument {
                    id: filename.clone(),
                    content: current_lines.join("\n"),
                    source,
                });
            }
        }
    }

    debug!(
        path = %path.display(),
        documents = documents.len(),
        total_rows = total_rows,
        csv_format = is_csv_format,
        "Read parquet file"
    );

    Ok(documents)
}

/// Read documents from S3 parquet files.
///
/// Downloads parquet files from the given `s3://bucket/prefix` URI to a
/// temporary directory, then reads them using the local reading logic.
async fn read_documents_from_s3(location: &str) -> anyhow::Result<Vec<SampledDocument>> {
    // Parse s3://bucket/prefix
    let without_scheme = location
        .strip_prefix("s3://")
        .ok_or_else(|| anyhow::anyhow!("Invalid S3 URI: {}", location))?;

    let (bucket, prefix) = match without_scheme.find('/') {
        Some(pos) => (&without_scheme[..pos], &without_scheme[pos + 1..]),
        None => (without_scheme, ""),
    };

    info!(bucket = bucket, prefix = prefix, "Listing S3 parquet files");

    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let s3_client = aws_sdk_s3::Client::new(&aws_config);

    // List objects with prefix
    let mut parquet_keys: Vec<String> = Vec::new();
    let mut continuation_token: Option<String> = None;

    loop {
        let mut request = s3_client
            .list_objects_v2()
            .bucket(bucket)
            .prefix(prefix);

        if let Some(token) = continuation_token.take() {
            request = request.continuation_token(token);
        }

        let response = request.send().await?;

        for obj in response.contents() {
            if let Some(key) = obj.key() {
                if key.ends_with(".parquet") {
                    parquet_keys.push(key.to_string());
                }
            }
        }

        if response.is_truncated() == Some(true) {
            continuation_token = response.next_continuation_token().map(|s| s.to_string());
        } else {
            break;
        }
    }

    if parquet_keys.is_empty() {
        anyhow::bail!(
            "No parquet files found at s3://{}/{}",
            bucket,
            prefix
        );
    }

    info!(
        count = parquet_keys.len(),
        "Found S3 parquet files for sampling"
    );

    // Download to temp directory and read
    let temp_dir = tempfile::TempDir::new()?;
    let mut all_documents = Vec::new();

    for key in &parquet_keys {
        let filename = key
            .rsplit('/')
            .next()
            .unwrap_or(key);
        let local_path = temp_dir.path().join(filename);

        debug!(key = key, local = %local_path.display(), "Downloading S3 parquet file");

        let response = s3_client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await?;

        let bytes = response.body.collect().await?.into_bytes();
        std::fs::write(&local_path, &bytes)?;

        let file_docs = read_single_parquet(&local_path)?;
        all_documents.extend(file_docs);
    }

    info!(
        total_documents = all_documents.len(),
        "Read documents from S3 parquet files"
    );

    Ok(all_documents)
}
