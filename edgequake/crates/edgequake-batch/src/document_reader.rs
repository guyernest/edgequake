//! Text/markdown file reading with front matter extraction.
//!
//! Reads `.txt` and `.md` files into `ReconstructedDocument` values, the same
//! type produced by the parquet reader. Markdown front matter (YAML between
//! `---` delimiters) is stripped and parsed into structured metadata.

use crate::document_splitter;
use crate::parquet_reader::{compute_sha256, ReconstructedDocument};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::warn;

/// Maximum file size allowed (100 MB).
const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;

/// Parsed YAML front matter from a markdown file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontMatter {
    pub title: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
    pub tags: Option<Vec<String>>,
    /// Additional fields not covered by the named fields above.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Structured metadata for a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub relative_path: String,
    pub heading_context: Option<String>,
    pub document_index: Option<usize>,
    pub front_matter: Option<FrontMatter>,
    pub content_hash: String,
}

/// Result of stripping front matter from content.
#[derive(Debug, Clone)]
pub struct FrontMatterResult {
    /// Parsed YAML metadata as a JSON value, or None if no front matter.
    pub metadata: Option<serde_json::Value>,
    /// Content with front matter removed.
    pub content: String,
}

/// Strip YAML front matter from the beginning of a document.
///
/// Front matter must start on line 1 with exactly `---`. The closing `---` ends
/// the front matter block. YAML between the delimiters is parsed with serde_yaml.
/// If no valid front matter is found, the input is returned unchanged with None metadata.
/// Handles both `\n` and `\r\n` line endings.
pub fn strip_front_matter(input: &str) -> FrontMatterResult {
    // Normalize line endings for consistent processing
    let normalized = input.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.lines().collect();

    // Front matter must start on line 1 with exactly "---"
    if lines.is_empty() || lines[0].trim() != "---" {
        return FrontMatterResult {
            metadata: None,
            content: input.to_string(),
        };
    }

    // Find the closing "---"
    let mut closing_idx = None;
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == "---" {
            closing_idx = Some(i);
            break;
        }
    }

    let Some(closing_idx) = closing_idx else {
        // No closing --- found, treat entire file as content
        return FrontMatterResult {
            metadata: None,
            content: input.to_string(),
        };
    };

    // Extract YAML between the delimiters
    let yaml_text: String = lines[1..closing_idx].join("\n");

    // Parse YAML into a serde_json::Value
    let metadata = match serde_yaml::from_str::<serde_json::Value>(&yaml_text) {
        Ok(value) => Some(value),
        Err(_) => {
            // Invalid YAML, treat as no front matter
            return FrontMatterResult {
                metadata: None,
                content: input.to_string(),
            };
        }
    };

    // Content is everything after the closing ---
    let remaining = if closing_idx + 1 < lines.len() {
        lines[closing_idx + 1..].join("\n")
    } else {
        String::new()
    };

    FrontMatterResult {
        metadata,
        content: remaining.trim_start().to_string(),
    }
}

/// Read a text file into a `ReconstructedDocument`.
///
/// Computes relative path from `base_dir` and SHA-256 content hash.
pub fn read_text_file(path: &Path, base_dir: &Path) -> anyhow::Result<ReconstructedDocument> {
    check_file_size(path)?;

    let content = std::fs::read_to_string(path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read '{}': {} (is it UTF-8 encoded?)",
            path.display(),
            e
        )
    })?;

    let relative = path
        .strip_prefix(base_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    let content_hash = compute_sha256(&content);

    Ok(ReconstructedDocument {
        filename: relative,
        content,
        content_hash,
        source_row_count: 1,
    })
}

/// Read a markdown file, stripping front matter.
///
/// Returns the `ReconstructedDocument` (with front-matter-stripped content)
/// and the parsed `FrontMatter` if present.
pub fn read_markdown_file(
    path: &Path,
    base_dir: &Path,
) -> anyhow::Result<(ReconstructedDocument, Option<FrontMatter>)> {
    check_file_size(path)?;

    let raw = std::fs::read_to_string(path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read '{}': {} (is it UTF-8 encoded?)",
            path.display(),
            e
        )
    })?;

    let fm_result = strip_front_matter(&raw);

    // Parse FrontMatter from the JSON value if present
    let front_matter = fm_result
        .metadata
        .and_then(|val| serde_json::from_value::<FrontMatter>(val).ok());

    let relative = path
        .strip_prefix(base_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();

    let content_hash = compute_sha256(&fm_result.content);

    let doc = ReconstructedDocument {
        filename: relative,
        content: fm_result.content,
        content_hash,
        source_row_count: 1,
    };

    Ok((doc, front_matter))
}

/// Read a single file, dispatching by extension.
///
/// Optionally splits multi-document files using the specified delimiter.
/// When `no_split` is true, splitting is skipped entirely.
pub fn read_single_file(
    path: &Path,
    base_dir: &Path,
    delimiter: Option<&str>,
    no_split: bool,
) -> anyhow::Result<Vec<ReconstructedDocument>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    match ext {
        "txt" => {
            let doc = read_text_file(path, base_dir)?;
            if no_split {
                return Ok(vec![doc]);
            }
            maybe_split(doc, delimiter, false)
        }
        "md" | "markdown" => {
            let (doc, _front_matter) = read_markdown_file(path, base_dir)?;
            if no_split {
                return Ok(vec![doc]);
            }
            maybe_split(doc, delimiter, true)
        }
        _ => {
            anyhow::bail!("Unsupported file type: .{}", ext);
        }
    }
}

/// Read all documents from a list of file paths.
///
/// On UTF-8 errors, logs a warning and skips the file (does not abort).
/// Returns accumulated documents sorted by filename.
pub fn read_all_documents(
    files: &[PathBuf],
    base_dir: &Path,
    delimiter: Option<&str>,
    no_split: bool,
) -> anyhow::Result<Vec<ReconstructedDocument>> {
    let mut all_docs: Vec<ReconstructedDocument> = Vec::new();

    for file in files {
        match read_single_file(file, base_dir, delimiter, no_split) {
            Ok(docs) => all_docs.extend(docs),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("UTF-8")
                    || msg.contains("utf-8")
                    || msg.contains("stream did not contain valid UTF-8")
                {
                    warn!(
                        path = %file.display(),
                        error = %e,
                        "Skipping file due to UTF-8 error"
                    );
                } else {
                    warn!(
                        path = %file.display(),
                        error = %e,
                        "Skipping file due to error"
                    );
                }
            }
        }
    }

    all_docs.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(all_docs)
}

/// Check file size and bail if it exceeds the limit.
fn check_file_size(path: &Path) -> anyhow::Result<()> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_FILE_SIZE {
        anyhow::bail!(
            "File exceeds 100MB limit: {} ({} bytes)",
            path.display(),
            metadata.len()
        );
    }
    Ok(())
}

/// Optionally split a document by delimiter.
fn maybe_split(
    doc: ReconstructedDocument,
    delimiter: Option<&str>,
    is_markdown: bool,
) -> anyhow::Result<Vec<ReconstructedDocument>> {
    if let Some(delim) = delimiter {
        Ok(document_splitter::split_by_delimiter(
            &doc.content,
            delim,
            &doc.filename,
        ))
    } else {
        // Try auto-detect
        if let Some(detected) = document_splitter::auto_detect_delimiter(&doc.content, is_markdown)
        {
            Ok(document_splitter::split_by_delimiter(
                &doc.content,
                &detected,
                &doc.filename,
            ))
        } else {
            Ok(vec![doc])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_strip_front_matter_basic() {
        let input = "---\ntitle: Hello World\nauthor: Test Author\n---\nBody content here.";
        let result = strip_front_matter(input);

        assert!(result.metadata.is_some());
        let meta = result.metadata.unwrap();
        assert_eq!(meta["title"], "Hello World");
        assert_eq!(meta["author"], "Test Author");
        assert_eq!(result.content, "Body content here.");
    }

    #[test]
    fn test_strip_front_matter_none() {
        let input = "No front matter here.\nJust regular content.";
        let result = strip_front_matter(input);

        assert!(result.metadata.is_none());
        assert_eq!(result.content, input);
    }

    #[test]
    fn test_strip_front_matter_no_closing() {
        let input = "---\ntitle: Incomplete\nNo closing delimiter";
        let result = strip_front_matter(input);

        // No closing --- means treat entire content as content
        assert!(result.metadata.is_none());
        assert_eq!(result.content, input);
    }

    #[test]
    fn test_read_text_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut f = std::fs::File::create(&file_path).unwrap();
        write!(f, "Hello, world!").unwrap();

        let doc = read_text_file(&file_path, dir.path()).unwrap();
        assert_eq!(doc.filename, "test.txt");
        assert_eq!(doc.content, "Hello, world!");
        assert!(!doc.content_hash.is_empty());
        assert_eq!(doc.source_row_count, 1);

        // Hash should be deterministic
        let doc2 = read_text_file(&file_path, dir.path()).unwrap();
        assert_eq!(doc.content_hash, doc2.content_hash);
    }

    #[test]
    fn test_read_markdown_file_with_front_matter() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("post.md");
        let content = "---\ntitle: My Post\nauthor: Alice\ntags:\n  - rust\n  - coding\n---\n\n# Introduction\n\nThis is the body.";
        std::fs::write(&file_path, content).unwrap();

        let (doc, front_matter) = read_markdown_file(&file_path, dir.path()).unwrap();

        // Content should exclude front matter
        assert!(!doc.content.contains("title:"));
        assert!(doc.content.contains("# Introduction"));
        assert!(doc.content.contains("This is the body."));

        // Front matter should be parsed
        let fm = front_matter.unwrap();
        assert_eq!(fm.title.as_deref(), Some("My Post"));
        assert_eq!(fm.author.as_deref(), Some("Alice"));
        let tags = fm.tags.unwrap();
        assert_eq!(tags, vec!["rust".to_string(), "coding".to_string()]);
    }
}
