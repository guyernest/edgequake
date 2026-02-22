//! Delimiter-based document splitting with auto-detect.
//!
//! Splits a single file containing multiple documents separated by delimiter
//! lines into individual `ReconstructedDocument` values. Supports explicit
//! delimiter specification and automatic detection of common patterns.

use crate::parquet_reader::{compute_sha256, ReconstructedDocument};

/// Split content on lines where `line.trim() == delimiter` (literal string match).
///
/// Each segment becomes a `ReconstructedDocument` with `filename#N` identifiers
/// (0-indexed). If only one segment is found, returns a single document with the
/// original filename (no index suffix). Empty segments (whitespace-only between
/// delimiters) are skipped.
pub fn split_by_delimiter(
    content: &str,
    delimiter: &str,
    filename: &str,
) -> Vec<ReconstructedDocument> {
    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();

    for line in content.lines() {
        if line.trim() == delimiter {
            // End of segment
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                segments.push(trimmed);
            }
            current.clear();
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }

    // Don't forget the last segment
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        segments.push(trimmed);
    }

    // If only one segment, return with original filename
    if segments.len() <= 1 {
        let content_text = segments.into_iter().next().unwrap_or_default();
        if content_text.is_empty() {
            return Vec::new();
        }
        let content_hash = compute_sha256(&content_text);
        return vec![ReconstructedDocument {
            filename: filename.to_string(),
            content: content_text,
            content_hash,
            source_row_count: 1,
        }];
    }

    // Multiple segments: use filename#N identifiers (0-indexed)
    segments
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let content_hash = compute_sha256(&text);
            ReconstructedDocument {
                filename: format!("{}#{}", filename, i),
                content: text,
                content_hash,
                source_row_count: 1,
            }
        })
        .collect()
}

/// Auto-detect common delimiter patterns in content.
///
/// Scans the first 10KB of content looking for lines that are exactly one of:
/// `***`, `* * *`, `---` (3+ dashes), `===` (3+ equals), `___` (3+ underscores).
///
/// For markdown files (`is_markdown=true`), skips `---` and `***` patterns to
/// avoid false positives from valid markdown horizontal rules and front matter.
///
/// Returns the delimiter if any pattern appears 2+ times; otherwise returns None.
///
/// IMPORTANT: Strips front matter before scanning to avoid confusing the closing
/// `---` of front matter with a delimiter.
pub fn auto_detect_delimiter(content: &str, is_markdown: bool) -> Option<String> {
    use crate::document_reader::strip_front_matter;

    // Strip front matter first to avoid false positives
    let result = strip_front_matter(content);
    let scan_content = &result.content;

    // Only scan first 10KB
    let scan_limit = scan_content.len().min(10 * 1024);
    let scan_slice = &scan_content[..scan_limit];

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for line in scan_slice.lines() {
        let trimmed = line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Check for known delimiter patterns
        let pattern = if trimmed == "***" {
            if is_markdown {
                continue; // Skip for markdown (valid HR)
            }
            Some("***".to_string())
        } else if trimmed == "* * *" {
            Some("* * *".to_string())
        } else if !trimmed.is_empty()
            && trimmed.chars().all(|c| c == '-')
            && trimmed.len() >= 3
        {
            if is_markdown {
                continue; // Skip for markdown (valid HR / front matter)
            }
            Some(trimmed.to_string())
        } else if !trimmed.is_empty()
            && trimmed.chars().all(|c| c == '=')
            && trimmed.len() >= 3
        {
            Some(trimmed.to_string())
        } else if !trimmed.is_empty()
            && trimmed.chars().all(|c| c == '_')
            && trimmed.len() >= 3
        {
            Some(trimmed.to_string())
        } else {
            None
        };

        if let Some(p) = pattern {
            *counts.entry(p).or_insert(0) += 1;
        }
    }

    // Return the first pattern that appears 2+ times
    // Sort by count descending, then alphabetically for determinism
    let mut candidates: Vec<(String, usize)> = counts.into_iter().collect();
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    candidates
        .into_iter()
        .find(|(_, count)| *count >= 2)
        .map(|(pattern, _)| pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_by_delimiter_basic() {
        let content = "doc1\n***\ndoc2\n***\ndoc3";
        let docs = split_by_delimiter(content, "***", "test.txt");
        assert_eq!(docs.len(), 3);
        assert_eq!(docs[0].filename, "test.txt#0");
        assert_eq!(docs[0].content, "doc1");
        assert_eq!(docs[1].filename, "test.txt#1");
        assert_eq!(docs[1].content, "doc2");
        assert_eq!(docs[2].filename, "test.txt#2");
        assert_eq!(docs[2].content, "doc3");
    }

    #[test]
    fn test_split_by_delimiter_single_doc() {
        let content = "just one document with no delimiter";
        let docs = split_by_delimiter(content, "***", "single.txt");
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].filename, "single.txt");
        assert_eq!(docs[0].content, "just one document with no delimiter");
    }

    #[test]
    fn test_split_empty_segments() {
        let content = "doc1\n***\n\n***\ndoc3";
        let docs = split_by_delimiter(content, "***", "test.txt");
        // Empty segment between the two *** lines is skipped
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].filename, "test.txt#0");
        assert_eq!(docs[0].content, "doc1");
        assert_eq!(docs[1].filename, "test.txt#1");
        assert_eq!(docs[1].content, "doc3");
    }

    #[test]
    fn test_auto_detect_delimiter_stars() {
        let content = "first doc\n***\nsecond doc\n***\nthird doc\n***\nfourth";
        let detected = auto_detect_delimiter(content, false);
        assert_eq!(detected, Some("***".to_string()));
    }

    #[test]
    fn test_auto_detect_markdown_skips_dashes() {
        let content = "heading\n---\nparagraph\n---\nanother";
        let detected = auto_detect_delimiter(content, true);
        // For markdown, --- should NOT be auto-detected (valid HR / front matter)
        assert_eq!(detected, None);
    }
}
