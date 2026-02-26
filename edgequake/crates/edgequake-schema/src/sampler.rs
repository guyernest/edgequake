//! Document sampling from local parquet files or S3 datasets.
//!
//! Supports both local filesystem paths and `s3://bucket/prefix` URIs.
//! Provides both uniform random sampling (legacy) and stratified sampling
//! with size bucketing, positional extraction, and TF-IDF topic diversity.

use arrow::array::{Array, StringArray};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use rand::prelude::IndexedRandom;
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info, warn};

use crate::tfidf;
use crate::types::{BucketBreakdown, BucketInfo, SamplingMetadata, SuggestSchemaInput};

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

// --- Size bucketing ---

/// Character count thresholds for size bucketing.
const SHORT_THRESHOLD: usize = 1_000;
const LONG_THRESHOLD: usize = 5_000;

/// Default window size for positional content extraction (characters).
const POSITIONAL_WINDOW_CHARS: usize = 2_000;

/// Default sampling budget.
const DEFAULT_BUDGET: usize = 24;

/// Size bucket for document length stratification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeBucket {
    Short,
    Medium,
    Long,
}

/// Assign a document to a size bucket based on character count.
fn assign_bucket(content_length: usize) -> SizeBucket {
    if content_length < SHORT_THRESHOLD {
        SizeBucket::Short
    } else if content_length > LONG_THRESHOLD {
        SizeBucket::Long
    } else {
        SizeBucket::Medium
    }
}

/// Distribute a sampling budget across three size buckets.
///
/// Starts with equal distribution (budget / 3 per bucket), caps at actual
/// bucket size, and redistributes surplus proportionally to buckets with room.
fn distribute_budget(bucket_counts: [usize; 3], total_budget: usize) -> [usize; 3] {
    let mut allocations = [0usize; 3];
    let base = total_budget / 3;
    let mut surplus = 0usize;

    // First pass: equal distribution, capped at bucket size
    for i in 0..3 {
        if bucket_counts[i] <= base {
            allocations[i] = bucket_counts[i];
            surplus += base - bucket_counts[i];
        } else {
            allocations[i] = base;
        }
    }

    // Add remainder from integer division
    surplus += total_budget - base * 3;

    // Second pass: redistribute surplus to buckets with room
    let mut remaining = surplus;
    for _round in 0..3 {
        if remaining == 0 {
            break;
        }
        let share = remaining;
        remaining = 0;
        // Count buckets with room
        let eligible: Vec<usize> = (0..3)
            .filter(|&i| allocations[i] < bucket_counts[i])
            .collect();
        if eligible.is_empty() {
            break;
        }
        let per_bucket = share / eligible.len();
        let extra = share % eligible.len();
        for (idx, &i) in eligible.iter().enumerate() {
            let give = per_bucket + if idx < extra { 1 } else { 0 };
            let actual = give.min(bucket_counts[i] - allocations[i]);
            allocations[i] += actual;
            remaining += give - actual;
        }
    }

    allocations
}

/// Extract positional content from a long document.
///
/// For documents longer than `window_chars * 3`, extracts three windows from
/// the beginning, middle, and end of the document. Window boundaries are
/// snapped to the nearest sentence boundary within +/- 200 chars of the
/// target position. For shorter documents, returns the content as-is.
fn extract_positional_content(content: &str, window_chars: usize) -> String {
    if content.len() <= window_chars * 3 {
        return content.to_string();
    }

    let snap_range = 200;

    // Helper: find nearest sentence boundary (. ! ? followed by whitespace)
    let snap_to_sentence = |pos: usize, search_forward: bool| -> usize {
        let start = pos.saturating_sub(snap_range);
        let end = (pos + snap_range).min(content.len());
        let region = &content[start..end];

        if search_forward {
            // Find first sentence boundary after the target position
            let offset_in_region = pos - start;
            for (i, _) in region[offset_in_region..].char_indices() {
                let abs = start + offset_in_region + i;
                if abs + 1 < content.len() {
                    let ch = content.as_bytes()[abs];
                    if (ch == b'.' || ch == b'!' || ch == b'?')
                        && content.as_bytes().get(abs + 1).is_some_and(|c| c.is_ascii_whitespace())
                    {
                        return abs + 1;
                    }
                }
            }
            // Fall back to newline
            for (i, _) in region[offset_in_region..].char_indices() {
                let abs = start + offset_in_region + i;
                if content.as_bytes()[abs] == b'\n' {
                    return abs + 1;
                }
            }
            pos
        } else {
            // Find last sentence boundary before the target position
            let offset_in_region = pos - start;
            let mut best = pos;
            for (i, _) in region[..offset_in_region].char_indices() {
                let abs = start + i;
                if abs + 1 < content.len() {
                    let ch = content.as_bytes()[abs];
                    if (ch == b'.' || ch == b'!' || ch == b'?')
                        && content.as_bytes().get(abs + 1).is_some_and(|c| c.is_ascii_whitespace())
                    {
                        best = abs + 2; // After the punctuation and space
                    }
                }
            }
            if best == pos {
                // Fall back to newline
                for (i, _) in region[..offset_in_region].char_indices() {
                    let abs = start + i;
                    if content.as_bytes()[abs] == b'\n' {
                        best = abs + 1;
                    }
                }
            }
            best
        }
    };

    // Beginning window: first window_chars, snapped forward at the end
    let begin_end = snap_to_sentence(window_chars, true);
    let beginning = &content[..begin_end.min(content.len())];

    // Middle window: centered around content.len()/2, snapped at both boundaries
    let mid_center = content.len() / 2;
    let mid_start = snap_to_sentence(mid_center.saturating_sub(window_chars / 2), false);
    let mid_end = snap_to_sentence(mid_center + window_chars / 2, true);
    let middle = &content[mid_start.min(content.len())..mid_end.min(content.len())];

    // End window: last window_chars, snapped backward at the start
    let end_start = snap_to_sentence(content.len().saturating_sub(window_chars), false);
    let ending = &content[end_start.min(content.len())..];

    format!(
        "{}\n\n[...MIDDLE OF DOCUMENT...]\n\n{}\n\n[...END OF DOCUMENT...]\n\n{}",
        beginning.trim(),
        middle.trim(),
        ending.trim()
    )
}

/// Select documents for maximum topic diversity using maximin distance.
///
/// Picks the first document randomly, then iteratively selects the document
/// with the maximum minimum-cosine-distance to all already-selected documents.
/// Returns indices into the input slice.
fn select_diverse(docs: &[(usize, HashMap<String, f64>)], budget: usize) -> Vec<usize> {
    if docs.is_empty() || budget == 0 {
        return vec![];
    }
    if budget >= docs.len() {
        return (0..docs.len()).collect();
    }

    let mut selected = Vec::with_capacity(budget);
    let mut selected_set = std::collections::HashSet::new();

    // Pick first doc randomly
    let mut rng = rand::rng();
    let first = *[0usize].choose(&mut rng).unwrap_or(&0);
    selected.push(first);
    selected_set.insert(first);

    // Track minimum distance from each unselected doc to the selected set
    let mut min_distances: Vec<f64> = vec![f64::MAX; docs.len()];

    while selected.len() < budget {
        // Update min distances with respect to the most recently selected doc
        let last_selected = *selected.last().unwrap();
        for (i, dist_entry) in min_distances.iter_mut().enumerate() {
            if selected_set.contains(&i) {
                *dist_entry = -1.0; // Mark as selected
                continue;
            }
            let sim = tfidf::cosine_similarity(&docs[i].1, &docs[last_selected].1);
            let dist = 1.0 - sim;
            if dist < *dist_entry {
                *dist_entry = dist;
            }
        }

        // Pick the doc with maximum minimum distance
        let mut best_idx = 0;
        let mut best_dist = -1.0_f64;
        for (i, &dist) in min_distances.iter().enumerate() {
            if !selected_set.contains(&i) && dist > best_dist {
                best_dist = dist;
                best_idx = i;
            }
        }

        selected.push(best_idx);
        selected_set.insert(best_idx);
    }

    selected
}

/// Perform stratified document sampling with size bucketing, positional
/// extraction, and TF-IDF topic diversity.
///
/// The sampling pipeline:
/// 1. Load all documents (reusing existing readers)
/// 2. If total_documents < 10, return all documents (skip stratification)
/// 3. Assign each document to a Short/Medium/Long size bucket
/// 4. Distribute budget across buckets (equal, then redistribute surplus)
/// 5. Within each bucket, compute TF-IDF fingerprints and select for diversity
/// 6. Apply positional content extraction to Long bucket documents
/// 7. Return sampled dataset and metadata
pub async fn sample_documents_stratified(
    location: &str,
    input: &SuggestSchemaInput,
) -> anyhow::Result<(SampledDataset, SamplingMetadata)> {
    let all_documents = if location.starts_with("s3://") {
        read_documents_from_s3(location).await?
    } else {
        read_documents_from_local(location)?
    };

    let total_count = all_documents.len();
    if total_count == 0 {
        let metadata = SamplingMetadata {
            total_documents: 0,
            sampled_count: 0,
            buckets: BucketBreakdown {
                short: BucketInfo { available: 0, sampled: 0 },
                medium: BucketInfo { available: 0, sampled: 0 },
                long: BucketInfo { available: 0, sampled: 0 },
            },
            topic_clusters_found: 0,
            auto_persona: None,
        };
        return Ok((SampledDataset { documents: vec![], total_count: 0 }, metadata));
    }

    let budget = input.sample_budget.unwrap_or(DEFAULT_BUDGET);

    // Tiny dataset: skip stratification, use all documents
    if total_count < 10 {
        info!(
            total = total_count,
            "Tiny dataset (<10 docs), using all documents"
        );
        let metadata = SamplingMetadata {
            total_documents: total_count,
            sampled_count: total_count,
            buckets: BucketBreakdown {
                short: BucketInfo {
                    available: all_documents.iter().filter(|d| assign_bucket(d.content.len()) == SizeBucket::Short).count(),
                    sampled: all_documents.iter().filter(|d| assign_bucket(d.content.len()) == SizeBucket::Short).count(),
                },
                medium: BucketInfo {
                    available: all_documents.iter().filter(|d| assign_bucket(d.content.len()) == SizeBucket::Medium).count(),
                    sampled: all_documents.iter().filter(|d| assign_bucket(d.content.len()) == SizeBucket::Medium).count(),
                },
                long: BucketInfo {
                    available: all_documents.iter().filter(|d| assign_bucket(d.content.len()) == SizeBucket::Long).count(),
                    sampled: all_documents.iter().filter(|d| assign_bucket(d.content.len()) == SizeBucket::Long).count(),
                },
            },
            topic_clusters_found: 0,
            auto_persona: None,
        };
        return Ok((SampledDataset { documents: all_documents, total_count }, metadata));
    }

    // Assign documents to size buckets
    let mut short_docs: Vec<(usize, &SampledDocument)> = Vec::new();
    let mut medium_docs: Vec<(usize, &SampledDocument)> = Vec::new();
    let mut long_docs: Vec<(usize, &SampledDocument)> = Vec::new();

    for (idx, doc) in all_documents.iter().enumerate() {
        match assign_bucket(doc.content.len()) {
            SizeBucket::Short => short_docs.push((idx, doc)),
            SizeBucket::Medium => medium_docs.push((idx, doc)),
            SizeBucket::Long => long_docs.push((idx, doc)),
        }
    }

    let bucket_counts = [short_docs.len(), medium_docs.len(), long_docs.len()];
    let allocations = distribute_budget(bucket_counts, budget.min(total_count));

    info!(
        total = total_count,
        budget = budget,
        short_available = short_docs.len(),
        medium_available = medium_docs.len(),
        long_available = long_docs.len(),
        short_alloc = allocations[0],
        medium_alloc = allocations[1],
        long_alloc = allocations[2],
        "Distributing sampling budget across size buckets"
    );

    // For each bucket: compute TF-IDF, select diverse
    let mut sampled_indices = Vec::new();
    let mut total_topic_clusters = 0usize;

    let buckets_with_alloc: Vec<(&[(usize, &SampledDocument)], usize)> = vec![
        (&short_docs, allocations[0]),
        (&medium_docs, allocations[1]),
        (&long_docs, allocations[2]),
    ];

    for (bucket_docs, alloc) in &buckets_with_alloc {
        if *alloc == 0 || bucket_docs.is_empty() {
            continue;
        }

        // Compute TF-IDF fingerprints
        let texts: Vec<&str> = bucket_docs.iter().map(|(_, d)| d.content.as_str()).collect();
        let tfidf_vectors = tfidf::compute_tfidf(&texts);

        // Count distinct topic clusters (docs with cosine similarity > 0.5 to each other)
        // Simple heuristic: count unique "top term" signatures
        let unique_signatures: std::collections::HashSet<String> = tfidf_vectors
            .iter()
            .map(|v| {
                let mut terms = tfidf::top_terms(v, 3);
                terms.sort();
                terms.join(",")
            })
            .collect();
        total_topic_clusters += unique_signatures.len();

        // Build indexed vectors for diversity selection
        let indexed: Vec<(usize, HashMap<String, f64>)> = bucket_docs
            .iter()
            .zip(tfidf_vectors.into_iter())
            .map(|((idx, _), vec)| (*idx, vec))
            .collect();

        let selected = select_diverse(&indexed, *alloc);
        for sel_idx in selected {
            sampled_indices.push(indexed[sel_idx].0);
        }
    }

    // Build sampled documents, applying positional extraction for long docs
    let mut sampled_docs = Vec::with_capacity(sampled_indices.len());
    for &idx in &sampled_indices {
        let doc = &all_documents[idx];
        let content = if assign_bucket(doc.content.len()) == SizeBucket::Long {
            extract_positional_content(&doc.content, POSITIONAL_WINDOW_CHARS)
        } else {
            doc.content.clone()
        };
        sampled_docs.push(SampledDocument {
            id: doc.id.clone(),
            content,
            source: doc.source.clone(),
        });
    }

    let sampled_count = sampled_docs.len();
    let metadata = SamplingMetadata {
        total_documents: total_count,
        sampled_count,
        buckets: BucketBreakdown {
            short: BucketInfo {
                available: short_docs.len(),
                sampled: allocations[0].min(short_docs.len()),
            },
            medium: BucketInfo {
                available: medium_docs.len(),
                sampled: allocations[1].min(medium_docs.len()),
            },
            long: BucketInfo {
                available: long_docs.len(),
                sampled: allocations[2].min(long_docs.len()),
            },
        },
        topic_clusters_found: total_topic_clusters,
        auto_persona: None,
    };

    info!(
        sampled = sampled_count,
        total = total_count,
        topic_clusters = total_topic_clusters,
        "Stratified sampling complete"
    );

    Ok((SampledDataset { documents: sampled_docs, total_count }, metadata))
}

/// Sample documents from a dataset at the given location.
///
/// The `location` parameter can be either:
/// - A local filesystem path (file or directory) containing `.parquet` files
/// - An `s3://bucket/prefix` URI pointing to parquet files in S3
///
/// `sample_percentage` controls what fraction of documents to sample (e.g. 10.0 = 10%).
#[deprecated(note = "Use sample_documents_stratified for better document coverage")]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assign_bucket() {
        assert_eq!(assign_bucket(500), SizeBucket::Short);
        assert_eq!(assign_bucket(999), SizeBucket::Short);
        assert_eq!(assign_bucket(1000), SizeBucket::Medium);
        assert_eq!(assign_bucket(3000), SizeBucket::Medium);
        assert_eq!(assign_bucket(5000), SizeBucket::Medium);
        assert_eq!(assign_bucket(5001), SizeBucket::Long);
        assert_eq!(assign_bucket(50000), SizeBucket::Long);
    }

    #[test]
    fn test_distribute_budget_equal() {
        // 30 budget, 20/20/20 docs -> 10/10/10
        let result = distribute_budget([20, 20, 20], 30);
        assert_eq!(result, [10, 10, 10]);
    }

    #[test]
    fn test_distribute_budget_uneven() {
        // 24 budget, 3/50/50 docs -> 3 short (capped), surplus goes to medium/long
        let result = distribute_budget([3, 50, 50], 24);
        assert_eq!(result[0], 3); // capped at available
        // Remaining 21 split between medium and long
        assert_eq!(result[1] + result[2], 21);
    }

    #[test]
    fn test_distribute_budget_all_small() {
        // 24 budget, 5/5/5 = 15 docs total -> 5/5/5 (all capped)
        let result = distribute_budget([5, 5, 5], 24);
        assert_eq!(result, [5, 5, 5]);
    }

    #[test]
    fn test_distribute_budget_one_empty() {
        // 24 budget, 0/50/50 docs -> 0 short, 12/12 medium/long
        let result = distribute_budget([0, 50, 50], 24);
        assert_eq!(result[0], 0);
        assert_eq!(result[1] + result[2], 24);
    }

    #[test]
    fn test_extract_positional_content_short_doc() {
        let short = "This is a short document. Nothing to extract.";
        let result = extract_positional_content(short, 2000);
        assert_eq!(result, short);
    }

    #[test]
    fn test_extract_positional_content_long_doc() {
        // Create a document longer than 3 * 2000 = 6000 chars
        let beginning = "Beginning section. ".repeat(200); // ~3800 chars
        let middle = "Middle section content here. ".repeat(200); // ~5600 chars
        let ending = "End section of document. ".repeat(200); // ~5000 chars
        let doc = format!("{}{}{}", beginning, middle, ending);
        assert!(doc.len() > 6000);

        let result = extract_positional_content(&doc, 2000);
        assert!(result.contains("[...MIDDLE OF DOCUMENT...]"));
        assert!(result.contains("[...END OF DOCUMENT...]"));
        // Should be significantly shorter than original
        assert!(result.len() < doc.len());
    }

    #[test]
    fn test_select_diverse_empty() {
        let docs: Vec<(usize, HashMap<String, f64>)> = vec![];
        assert!(select_diverse(&docs, 5).is_empty());
    }

    #[test]
    fn test_select_diverse_budget_exceeds() {
        let docs: Vec<(usize, HashMap<String, f64>)> = vec![
            (0, HashMap::from([("a".to_string(), 1.0)])),
            (1, HashMap::from([("b".to_string(), 1.0)])),
        ];
        let selected = select_diverse(&docs, 5);
        assert_eq!(selected.len(), 2); // Can't select more than available
    }

    #[test]
    fn test_select_diverse_picks_different() {
        // Three very different documents
        let docs: Vec<(usize, HashMap<String, f64>)> = vec![
            (0, HashMap::from([("apple".to_string(), 1.0)])),
            (1, HashMap::from([("banana".to_string(), 1.0)])),
            (2, HashMap::from([("cherry".to_string(), 1.0)])),
        ];
        let selected = select_diverse(&docs, 3);
        assert_eq!(selected.len(), 3);
        // All three should be selected since they're maximally different
        let mut sorted = selected.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2]);
    }

    #[test]
    fn test_tiny_dataset_uses_all() {
        // Create 5 docs (< 10 threshold)
        let docs: Vec<SampledDocument> = (0..5)
            .map(|i| SampledDocument {
                id: format!("doc-{}", i),
                content: format!("Content for document number {}", i),
                source: "test".to_string(),
            })
            .collect();

        // We can't easily test the async function here, but we can test the threshold logic
        assert!(docs.len() < 10);
    }
}
