//! Recursive directory traversal with glob filtering.
//!
//! Scans directories for supported file types (.txt, .md, .markdown),
//! skipping hidden files/directories and applying include/exclude glob patterns.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Result of scanning a directory for supported files.
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// Discovered .txt files.
    pub txt_files: Vec<PathBuf>,
    /// Discovered .md/.markdown files.
    pub md_files: Vec<PathBuf>,
    /// Number of files skipped (unsupported extensions).
    pub skipped_count: usize,
    /// Set of unsupported extensions encountered.
    pub skipped_extensions: HashSet<String>,
}

impl ScanResult {
    /// Print a summary of the scan results via `tracing::info!`.
    pub fn print_summary(&self) {
        let skipped_exts: Vec<&str> = self.skipped_extensions.iter().map(|s| s.as_str()).collect();
        info!(
            md_files = self.md_files.len(),
            txt_files = self.txt_files.len(),
            skipped = self.skipped_count,
            skipped_extensions = ?skipped_exts,
            "{} .md files, {} .txt files, {} skipped ({})",
            self.md_files.len(),
            self.txt_files.len(),
            self.skipped_count,
            skipped_exts.join(", ")
        );
    }

    /// Return all supported files (txt + md) combined, sorted by path.
    pub fn supported_files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = self
            .txt_files
            .iter()
            .chain(self.md_files.iter())
            .cloned()
            .collect();
        files.sort();
        files
    }
}

/// Builder for directory scanning with configurable options.
pub struct DirectoryScanner {
    root: PathBuf,
    recursive: bool,
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
}

impl DirectoryScanner {
    /// Create a new scanner rooted at the given path.
    ///
    /// Defaults: recursive=true, no include/exclude patterns.
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            recursive: true,
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
        }
    }

    /// Set whether to scan recursively (default: true).
    pub fn recursive(mut self, recursive: bool) -> Self {
        self.recursive = recursive;
        self
    }

    /// Set glob include patterns. Only files matching at least one pattern are included.
    pub fn include_patterns(mut self, patterns: &[String]) -> Self {
        self.include_patterns = patterns.to_vec();
        self
    }

    /// Set glob exclude patterns. Files matching any pattern are excluded.
    pub fn exclude_patterns(mut self, patterns: &[String]) -> Self {
        self.exclude_patterns = patterns.to_vec();
        self
    }

    /// Scan the directory and return results.
    pub fn scan(&self) -> anyhow::Result<ScanResult> {
        let mut result = ScanResult {
            txt_files: Vec::new(),
            md_files: Vec::new(),
            skipped_count: 0,
            skipped_extensions: HashSet::new(),
        };

        // Build glob sets
        let include_set = if !self.include_patterns.is_empty() {
            let mut builder = globset::GlobSetBuilder::new();
            for pattern in &self.include_patterns {
                builder.add(globset::Glob::new(pattern)?);
            }
            Some(builder.build()?)
        } else {
            None
        };

        let exclude_set = if !self.exclude_patterns.is_empty() {
            let mut builder = globset::GlobSetBuilder::new();
            for pattern in &self.exclude_patterns {
                builder.add(globset::Glob::new(pattern)?);
            }
            Some(builder.build()?)
        } else {
            None
        };

        // Walk directory
        let mut walker = walkdir::WalkDir::new(&self.root);
        if !self.recursive {
            walker = walker.max_depth(1);
        }

        for entry in walker
            .into_iter()
            .filter_entry(|e| {
                // Skip hidden files and directories (start with '.')
                e.file_name()
                    .to_str()
                    .map(|s| !s.starts_with('.'))
                    .unwrap_or(false)
            })
        {
            let entry = entry?;

            // Only process files
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path().to_path_buf();

            // Compute relative path for glob matching
            let rel_path = path
                .strip_prefix(&self.root)
                .unwrap_or(&path);
            let rel_str = rel_path.to_string_lossy();

            // Check exclude patterns first
            if let Some(ref exclude) = exclude_set {
                if exclude.is_match(rel_str.as_ref()) {
                    result.skipped_count += 1;
                    continue;
                }
            }

            // Check include patterns (skip if has includes and doesn't match)
            if let Some(ref include) = include_set {
                if !include.is_match(rel_str.as_ref()) {
                    result.skipped_count += 1;
                    continue;
                }
            }

            // Classify by extension
            match path.extension().and_then(|e| e.to_str()) {
                Some("txt") => result.txt_files.push(path),
                Some("md") | Some("markdown") => result.md_files.push(path),
                Some(ext) => {
                    warn!(path = %path.display(), extension = ext, "Skipping unsupported file type");
                    result.skipped_extensions.insert(ext.to_string());
                    result.skipped_count += 1;
                }
                None => {
                    warn!(path = %path.display(), "Skipping file with no extension");
                    result.skipped_count += 1;
                }
            }
        }

        // Sort for deterministic output
        result.txt_files.sort();
        result.md_files.sort();

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_scan_directory_basic() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file1.txt"), "hello").unwrap();
        fs::write(dir.path().join("file2.md"), "# heading").unwrap();
        fs::write(dir.path().join("data.csv"), "a,b,c").unwrap();

        let result = DirectoryScanner::new(dir.path()).scan().unwrap();
        assert_eq!(result.txt_files.len(), 1);
        assert_eq!(result.md_files.len(), 1);
        assert_eq!(result.skipped_count, 1);
        assert!(result.skipped_extensions.contains("csv"));
    }

    #[test]
    fn test_scan_skips_hidden_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("visible.txt"), "visible").unwrap();
        fs::write(dir.path().join(".hidden_file"), "hidden").unwrap();

        // Also create a hidden directory with a file inside
        let hidden_dir = dir.path().join(".hidden_dir");
        fs::create_dir(&hidden_dir).unwrap();
        fs::write(hidden_dir.join("inside.txt"), "inside hidden").unwrap();

        let result = DirectoryScanner::new(dir.path()).scan().unwrap();
        assert_eq!(result.txt_files.len(), 1);
        assert_eq!(result.txt_files[0].file_name().unwrap().to_str().unwrap(), "visible.txt");
    }

    #[test]
    fn test_scan_include_pattern() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file1.txt"), "text").unwrap();
        fs::write(dir.path().join("file2.md"), "markdown").unwrap();

        let result = DirectoryScanner::new(dir.path())
            .include_patterns(&["*.md".to_string()])
            .scan()
            .unwrap();

        assert_eq!(result.md_files.len(), 1);
        // .txt is skipped because it doesn't match include pattern
        assert_eq!(result.txt_files.len(), 0);
    }

    #[test]
    fn test_scan_exclude_pattern() {
        let dir = tempfile::tempdir().unwrap();
        let drafts = dir.path().join("drafts");
        fs::create_dir(&drafts).unwrap();
        fs::write(dir.path().join("good.md"), "keep").unwrap();
        fs::write(drafts.join("draft.md"), "skip").unwrap();

        let result = DirectoryScanner::new(dir.path())
            .exclude_patterns(&["drafts/**".to_string()])
            .scan()
            .unwrap();

        assert_eq!(result.md_files.len(), 1);
        assert!(result.md_files[0].ends_with("good.md"));
    }
}
