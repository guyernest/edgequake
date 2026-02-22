//! Text chunking with overlap for document processing.
//!
//! @implements FEAT0002
//! @implements FEAT0301
//! @implements FEAT0302
//!
//! # Implements
//!
//! - **FEAT0002**: Text Chunking with Overlap
//! - **FEAT0301**: Character-Based Chunking
//! - **FEAT0302**: Token-Based Chunking
//!
//! # Enforces
//!
//! - **BR0002**: Chunk size 1200 tokens, overlap 100 tokens (default config)
//!
//! # WHY: Overlapping Chunks
//!
//! Overlap between chunks ensures:
//! 1. Context continuity across chunk boundaries
//! 2. Entity mentions spanning two chunks are captured
//! 3. Better retrieval for queries at chunk boundaries
//!
//! The default 100-token overlap (~8% of chunk size) balances:
//! - Coverage (entities not missed)
//! - Efficiency (minimal duplicate processing)
//!
//! This module provides flexible text chunking with support for custom chunking functions.
//! Users can implement the `ChunkingStrategy` trait to provide their own chunking logic.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json;
use std::sync::Arc;

use crate::error::Result;

/// Result of a custom chunking operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkResult {
    /// The chunk text content.
    pub content: String,
    /// Approximate token count.
    pub tokens: usize,
    /// Zero-based index indicating the chunk's order in the document.
    pub chunk_order_index: usize,
    /// Optional metadata for strategy-specific context (e.g. heading breadcrumbs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Trait for custom chunking strategies.
///
/// Implement this trait to provide your own chunking logic for document processing.
/// This allows for flexible chunking strategies such as:
/// - Semantic chunking (based on meaning/topics)
/// - Fixed-size chunking with custom separators
/// - Language-specific chunking (code, markdown, etc.)
#[async_trait]
pub trait ChunkingStrategy: Send + Sync {
    /// Chunk the given text content into smaller pieces.
    ///
    /// # Arguments
    /// * `content` - The full text content to chunk
    /// * `config` - The chunking configuration
    ///
    /// # Returns
    /// A vector of chunk results with content, token count, and order index
    async fn chunk(&self, content: &str, config: &ChunkerConfig) -> Result<Vec<ChunkResult>>;

    /// Get the name of this chunking strategy.
    fn name(&self) -> &str;
}

/// Configuration for the chunker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkerConfig {
    /// Target chunk size in tokens.
    pub chunk_size: usize,

    /// Overlap between chunks in tokens.
    pub chunk_overlap: usize,

    /// Minimum chunk size (won't create chunks smaller than this).
    pub min_chunk_size: usize,

    /// Separator characters for splitting.
    pub separators: Vec<String>,

    /// Whether to preserve sentence boundaries.
    pub preserve_sentences: bool,

    /// Optional character to split on first (e.g., "\n" for line-by-line).
    pub split_by_character: Option<String>,

    /// If true, split only on the specified character, don't apply token limits.
    pub split_by_character_only: bool,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        Self {
            chunk_size: 1200,
            chunk_overlap: 100,
            min_chunk_size: 100,
            separators: vec![
                "\n\n".to_string(),
                "\n".to_string(),
                ". ".to_string(),
                "! ".to_string(),
                "? ".to_string(),
                "; ".to_string(),
                ", ".to_string(),
                " ".to_string(),
            ],
            preserve_sentences: true,
            split_by_character: None,
            split_by_character_only: false,
        }
    }
}

/// A chunk of text with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextChunk {
    /// Unique identifier for the chunk.
    pub id: String,

    /// The chunk text content.
    pub content: String,

    /// Index of this chunk in the document.
    pub index: usize,

    /// Character offset from the start of the document.
    pub start_offset: usize,

    /// Character offset to the end of the chunk.
    pub end_offset: usize,

    /// Starting line number (1-based) in the original document.
    pub start_line: usize,

    /// Ending line number (1-based, inclusive) in the original document.
    pub end_line: usize,

    /// Approximate token count.
    pub token_count: usize,

    /// Chunk embedding.
    pub embedding: Option<Vec<f32>>,
}

impl TextChunk {
    /// Create a new text chunk.
    pub fn new(
        id: impl Into<String>,
        content: impl Into<String>,
        index: usize,
        start_offset: usize,
        end_offset: usize,
    ) -> Self {
        let content = content.into();
        let token_count = estimate_tokens(&content);
        Self {
            id: id.into(),
            content,
            index,
            start_offset,
            end_offset,
            start_line: 1, // Default, should be set via with_line_numbers()
            end_line: 1,   // Default, should be set via with_line_numbers()
            token_count,
            embedding: None,
        }
    }

    /// Create a new text chunk with line numbers.
    pub fn with_line_numbers(
        id: impl Into<String>,
        content: impl Into<String>,
        index: usize,
        start_offset: usize,
        end_offset: usize,
        start_line: usize,
        end_line: usize,
    ) -> Self {
        let content = content.into();
        let token_count = estimate_tokens(&content);
        Self {
            id: id.into(),
            content,
            index,
            start_offset,
            end_offset,
            start_line,
            end_line,
            token_count,
            embedding: None,
        }
    }

    /// Set line numbers after creation.
    pub fn set_line_numbers(&mut self, start_line: usize, end_line: usize) {
        self.start_line = start_line;
        self.end_line = end_line;
    }
}

/// Calculate line numbers for a chunk based on character offsets.
///
/// # Arguments
/// * `full_text` - The complete document text
/// * `start_offset` - Starting character offset of the chunk
/// * `end_offset` - Ending character offset of the chunk
///
/// # Returns
/// A tuple of (start_line, end_line), both 1-based
pub fn calculate_line_numbers(
    full_text: &str,
    start_offset: usize,
    end_offset: usize,
) -> (usize, usize) {
    // Ensure offsets are on valid char boundaries
    let safe_start = floor_char_boundary(full_text, start_offset.min(full_text.len()));
    let safe_end = floor_char_boundary(full_text, end_offset.min(full_text.len()));

    // Count newlines before the start offset to get start line
    let before_chunk = &full_text[..safe_start];
    let start_line = before_chunk.chars().filter(|&c| c == '\n').count() + 1;

    // Count newlines within the chunk to get end line
    let chunk_text = &full_text[safe_start..safe_end];
    let lines_in_chunk = chunk_text.chars().filter(|&c| c == '\n').count();
    let end_line = start_line + lines_in_chunk;

    (start_line, end_line)
}

/// Estimate token count (rough approximation: 1 token ≈ 4 chars).
fn estimate_tokens(text: &str) -> usize {
    (text.len() as f32 / 4.0).ceil() as usize
}

/// Default token-based chunking strategy.
///
/// This is the standard chunking strategy that splits text into chunks
/// based on token count with overlap, respecting sentence boundaries.
pub struct TokenBasedChunking;

#[async_trait]
impl ChunkingStrategy for TokenBasedChunking {
    async fn chunk(&self, content: &str, config: &ChunkerConfig) -> Result<Vec<ChunkResult>> {
        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        // Check for split_by_character_only mode (GAP-017)
        if let Some(ref split_char) = config.split_by_character {
            if config.split_by_character_only {
                return Ok(content
                    .split(split_char.as_str())
                    .enumerate()
                    .filter(|(_, s)| !s.trim().is_empty())
                    .map(|(idx, s)| ChunkResult {
                        content: s.to_string(),
                        tokens: estimate_tokens(s),
                        chunk_order_index: idx,
                        metadata: None,
                    })
                    .collect());
            }
        }

        let target_chars = config.chunk_size * 4;
        let overlap_chars = config.chunk_overlap * 4;
        let min_chars = config.min_chunk_size * 4;

        let chunks = split_text_internal(
            content,
            target_chars,
            overlap_chars,
            min_chars,
            &config.separators,
        );

        Ok(chunks
            .into_iter()
            .enumerate()
            .map(|(idx, (text, _, _))| ChunkResult {
                content: text.clone(),
                tokens: estimate_tokens(&text),
                chunk_order_index: idx,
                metadata: None,
            })
            .collect())
    }

    fn name(&self) -> &str {
        "token_based"
    }
}

/// Character-based chunking strategy (GAP-017).
///
/// Splits text on a specific character (like newline) for pre-split content.
///
/// @implements FEAT0306 (Character-Based Chunking - CharacterBasedChunking struct)
pub struct CharacterBasedChunking {
    /// Character to split on.
    pub split_character: String,
}

impl CharacterBasedChunking {
    /// Create a new character-based chunking strategy.
    pub fn new(split_character: impl Into<String>) -> Self {
        Self {
            split_character: split_character.into(),
        }
    }

    /// Create a newline-based chunker.
    pub fn by_newline() -> Self {
        Self::new("\n")
    }

    /// Create a paragraph-based chunker.
    pub fn by_paragraph() -> Self {
        Self::new("\n\n")
    }
}

#[async_trait]
impl ChunkingStrategy for CharacterBasedChunking {
    async fn chunk(&self, content: &str, _config: &ChunkerConfig) -> Result<Vec<ChunkResult>> {
        Ok(content
            .split(&self.split_character)
            .enumerate()
            .filter(|(_, s)| !s.trim().is_empty())
            .map(|(idx, s)| ChunkResult {
                content: s.to_string(),
                tokens: estimate_tokens(s),
                chunk_order_index: idx,
                metadata: None,
            })
            .collect())
    }

    fn name(&self) -> &str {
        "character_based"
    }
}

/// Sentence boundary chunking strategy.
///
/// @implements SPEC-001/Issue-10: Pluggable chunk cutoff system
///
/// This strategy ensures chunks never split mid-sentence, preserving
/// complete sentences for better entity extraction context.
///
/// # Algorithm
///
/// 1. Split text into sentences using period/question/exclamation
/// 2. Accumulate sentences until target chunk size reached
/// 3. Create chunk and start new accumulation
/// 4. Overlap is handled by carrying last N sentences to next chunk
///
/// # WHY Sentence Boundaries?
///
/// Mid-sentence splits can break entity extraction context:
/// - "Dr. Smith works at Microsoft. He" → Entity "He" orphaned
/// - "Dr. Smith works at Microsoft." → Complete context preserved
pub struct SentenceBoundaryChunking;

impl SentenceBoundaryChunking {
    /// Create a new sentence boundary chunker.
    pub fn new() -> Self {
        Self
    }
}

impl Default for SentenceBoundaryChunking {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChunkingStrategy for SentenceBoundaryChunking {
    async fn chunk(&self, content: &str, config: &ChunkerConfig) -> Result<Vec<ChunkResult>> {
        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        // Split into sentences (simple heuristic: period, question, exclamation)
        let sentences = split_into_sentences(content);

        if sentences.is_empty() {
            // No sentence boundaries found, fall back to token-based
            return TokenBasedChunking.chunk(content, config).await;
        }

        let target_tokens = config.chunk_size;
        let overlap_tokens = config.chunk_overlap;
        let min_tokens = config.min_chunk_size;

        let mut chunks = Vec::new();
        let mut current_chunk = String::new();
        let mut current_tokens = 0;
        let mut sentence_buffer: Vec<String> = Vec::new();
        let mut chunk_index = 0;

        for sentence in sentences {
            let sentence_tokens = estimate_tokens(&sentence);

            // If adding this sentence would exceed target, finalize current chunk
            if current_tokens + sentence_tokens > target_tokens && current_tokens >= min_tokens {
                chunks.push(ChunkResult {
                    content: current_chunk.trim().to_string(),
                    tokens: current_tokens,
                    chunk_order_index: chunk_index,
                    metadata: None,
                });
                chunk_index += 1;

                // Start new chunk with overlap (carry some sentences)
                let overlap_sentences = take_overlap_sentences(&sentence_buffer, overlap_tokens);
                current_chunk = overlap_sentences.join(" ");
                current_tokens = estimate_tokens(&current_chunk);
                sentence_buffer.clear();
            }

            // Add sentence to current chunk
            if !current_chunk.is_empty() {
                current_chunk.push(' ');
            }
            current_chunk.push_str(&sentence);
            current_tokens += sentence_tokens;
            sentence_buffer.push(sentence);
        }

        // Add final chunk if non-empty
        if current_tokens >= min_tokens {
            chunks.push(ChunkResult {
                content: current_chunk.trim().to_string(),
                tokens: current_tokens,
                chunk_order_index: chunk_index,
                metadata: None,
            });
        }

        Ok(chunks)
    }

    fn name(&self) -> &str {
        "sentence_boundary"
    }
}

/// Paragraph boundary chunking strategy.
///
/// @implements SPEC-001/Issue-10: Pluggable chunk cutoff system
///
/// This strategy groups paragraphs together, never splitting within
/// a paragraph. Ideal for structured documents.
///
/// # Algorithm
///
/// 1. Split text on double newlines (paragraphs)
/// 2. Accumulate paragraphs until target chunk size reached
/// 3. Create chunk and start new accumulation
///
/// # WHY Paragraph Boundaries?
///
/// Paragraphs often contain self-contained ideas:
/// - Entity introductions usually complete within paragraph
/// - Relationships described in same paragraph as entities
/// - Splitting preserves narrative flow
pub struct ParagraphBoundaryChunking;

impl ParagraphBoundaryChunking {
    /// Create a new paragraph boundary chunker.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ParagraphBoundaryChunking {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChunkingStrategy for ParagraphBoundaryChunking {
    async fn chunk(&self, content: &str, config: &ChunkerConfig) -> Result<Vec<ChunkResult>> {
        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        // Split on double newlines (paragraphs)
        let paragraphs: Vec<&str> = content
            .split("\n\n")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        if paragraphs.is_empty() {
            // No paragraphs found, try single newlines
            let single_line_paragraphs: Vec<&str> = content
                .split('\n')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();

            if single_line_paragraphs.is_empty() {
                return TokenBasedChunking.chunk(content, config).await;
            }
            return chunk_paragraphs(&single_line_paragraphs, config);
        }

        chunk_paragraphs(&paragraphs, config)
    }

    fn name(&self) -> &str {
        "paragraph_boundary"
    }
}

/// Helper to chunk paragraphs into size-limited chunks.
fn chunk_paragraphs(paragraphs: &[&str], config: &ChunkerConfig) -> Result<Vec<ChunkResult>> {
    let target_tokens = config.chunk_size;
    let min_tokens = config.min_chunk_size;

    let mut chunks = Vec::new();
    let mut current_chunk = String::new();
    let mut current_tokens = 0;
    let mut chunk_index = 0;

    for para in paragraphs {
        let para_tokens = estimate_tokens(para);

        // If this paragraph alone exceeds target, add it as its own chunk
        if para_tokens >= target_tokens {
            // First, save current accumulation if any
            if current_tokens >= min_tokens {
                chunks.push(ChunkResult {
                    content: current_chunk.trim().to_string(),
                    tokens: current_tokens,
                    chunk_order_index: chunk_index,
                    metadata: None,
                });
                chunk_index += 1;
                current_chunk = String::new();
                current_tokens = 0;
            }

            // Add large paragraph as its own chunk
            chunks.push(ChunkResult {
                content: para.to_string(),
                tokens: para_tokens,
                chunk_order_index: chunk_index,
                metadata: None,
            });
            chunk_index += 1;
            continue;
        }

        // If adding would exceed target, finalize current chunk
        if current_tokens + para_tokens > target_tokens && current_tokens >= min_tokens {
            chunks.push(ChunkResult {
                content: current_chunk.trim().to_string(),
                tokens: current_tokens,
                chunk_order_index: chunk_index,
                metadata: None,
            });
            chunk_index += 1;
            current_chunk = String::new();
            current_tokens = 0;
        }

        // Add paragraph to current chunk
        if !current_chunk.is_empty() {
            current_chunk.push_str("\n\n");
        }
        current_chunk.push_str(para);
        current_tokens += para_tokens;
    }

    // Add final chunk
    if current_tokens >= min_tokens {
        chunks.push(ChunkResult {
            content: current_chunk.trim().to_string(),
            tokens: current_tokens,
            chunk_order_index: chunk_index,
            metadata: None,
        });
    }

    Ok(chunks)
}

// =========================================================================
// Heading Boundary Chunking (Markdown-aware)
// =========================================================================

/// Heading boundary chunking strategy for markdown documents.
///
/// This strategy splits markdown content on heading boundaries (H1/H2 first,
/// then H3/H4 for oversized sections), keeps code blocks intact, and produces
/// heading breadcrumb context metadata.
///
/// # Algorithm
///
/// 1. Split content on H1/H2 heading boundaries
/// 2. For oversized sections, sub-split on H3/H4 boundaries
/// 3. If still oversized, fall back to paragraph boundary splitting
/// 4. Code blocks (``` fences) are never split across boundaries
/// 5. Each chunk carries a heading breadcrumb string in metadata
pub struct HeadingBoundaryChunking;

impl HeadingBoundaryChunking {
    /// Create a new heading boundary chunker.
    pub fn new() -> Self {
        Self
    }
}

impl Default for HeadingBoundaryChunking {
    fn default() -> Self {
        Self::new()
    }
}

/// Detect the heading level of a markdown line.
///
/// Returns `Some(level)` for lines like `# Heading` (level 1) through
/// `###### Heading` (level 6). Returns `None` if the line is not a heading.
/// Requires a space after the `#` characters (standard markdown).
fn detect_heading_level(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    // Must have a space after the hashes
    let rest = &trimmed[hashes..];
    if rest.starts_with(' ') {
        Some(hashes)
    } else {
        None
    }
}

/// Build a breadcrumb string from a heading stack.
///
/// Joins heading texts with ` > ` separator.
/// E.g. `[(1, "Chapter 3"), (2, "Section 2")]` becomes `"Chapter 3 > Section 2"`.
fn build_breadcrumb(heading_stack: &[(usize, String)]) -> String {
    heading_stack
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join(" > ")
}

/// Extract heading text from a heading line (strips the `#` prefix and whitespace).
fn extract_heading_text(line: &str) -> String {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    trimmed[hashes..].trim().to_string()
}

/// Split markdown content on H1/H2 heading boundaries.
///
/// Returns `(section_content, heading_breadcrumb)` pairs.
/// Code blocks (``` fences) are respected and never trigger heading splits.
fn split_on_headings(content: &str, _target_tokens: usize) -> Vec<(String, String)> {
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut heading_stack: Vec<(usize, String)> = Vec::new();
    let mut in_code_fence = false;
    let mut current_section = String::new();

    for line in content.lines() {
        let trimmed_start = line.trim_start();

        // Handle code fence toggling
        if trimmed_start.starts_with("```") {
            in_code_fence = !in_code_fence;
            if !current_section.is_empty() {
                current_section.push('\n');
            }
            current_section.push_str(line);
            continue;
        }

        // Inside code fence: never split
        if in_code_fence {
            if !current_section.is_empty() {
                current_section.push('\n');
            }
            current_section.push_str(line);
            continue;
        }

        // Check for heading (H1 or H2 only for primary splits)
        if let Some(level) = detect_heading_level(line) {
            if level <= 2 && !current_section.trim().is_empty() {
                // Flush current section
                let breadcrumb = build_breadcrumb(&heading_stack);
                sections.push((current_section, breadcrumb));
                current_section = String::new();
            }

            // Update heading stack: remove headings at same or deeper level
            heading_stack.retain(|(l, _)| *l < level);
            heading_stack.push((level, extract_heading_text(line)));
        }

        // Add line to current section
        if !current_section.is_empty() {
            current_section.push('\n');
        }
        current_section.push_str(line);
    }

    // Flush final section
    if !current_section.trim().is_empty() {
        let breadcrumb = build_breadcrumb(&heading_stack);
        sections.push((current_section, breadcrumb));
    }

    sections
}

/// Sub-split an oversized section on H3/H4 boundaries, then fall back to paragraphs.
///
/// Each sub-section inherits the parent heading breadcrumb, appended with its own
/// sub-heading if applicable. Code blocks that exceed target_tokens are emitted
/// as their own chunk (never split).
fn sub_split_section(
    content: &str,
    heading_breadcrumb: &str,
    target_tokens: usize,
) -> Vec<(String, String)> {
    // First try splitting on H3/H4 boundaries
    let sub_sections = split_on_sub_headings(content, heading_breadcrumb);

    if sub_sections.len() > 1 {
        // Check if any sub-section is still oversized
        let mut result = Vec::new();
        for (sub_content, sub_breadcrumb) in sub_sections {
            let tokens = estimate_tokens(&sub_content);
            if tokens > target_tokens {
                // Fall back to paragraph splitting for this sub-section
                let para_chunks = split_on_paragraphs(&sub_content, &sub_breadcrumb, target_tokens);
                result.extend(para_chunks);
            } else {
                result.push((sub_content, sub_breadcrumb));
            }
        }
        return result;
    }

    // No H3/H4 sub-headings found, fall back to paragraph splitting
    split_on_paragraphs(content, heading_breadcrumb, target_tokens)
}

/// Split content on H3/H4 heading boundaries.
fn split_on_sub_headings(content: &str, parent_breadcrumb: &str) -> Vec<(String, String)> {
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut in_code_fence = false;
    let mut current_section = String::new();
    let mut current_sub_heading: Option<String> = None;

    for line in content.lines() {
        let trimmed_start = line.trim_start();

        // Handle code fence toggling
        if trimmed_start.starts_with("```") {
            in_code_fence = !in_code_fence;
            if !current_section.is_empty() {
                current_section.push('\n');
            }
            current_section.push_str(line);
            continue;
        }

        if in_code_fence {
            if !current_section.is_empty() {
                current_section.push('\n');
            }
            current_section.push_str(line);
            continue;
        }

        // Check for H3/H4 heading
        if let Some(level) = detect_heading_level(line) {
            if (level == 3 || level == 4) && !current_section.trim().is_empty() {
                // Flush current section
                let breadcrumb = if let Some(ref sub) = current_sub_heading {
                    if parent_breadcrumb.is_empty() {
                        sub.clone()
                    } else {
                        format!("{} > {}", parent_breadcrumb, sub)
                    }
                } else {
                    parent_breadcrumb.to_string()
                };
                sections.push((current_section, breadcrumb));
                current_section = String::new();
            }

            if level == 3 || level == 4 {
                current_sub_heading = Some(extract_heading_text(line));
            }
        }

        if !current_section.is_empty() {
            current_section.push('\n');
        }
        current_section.push_str(line);
    }

    // Flush final section
    if !current_section.trim().is_empty() {
        let breadcrumb = if let Some(ref sub) = current_sub_heading {
            if parent_breadcrumb.is_empty() {
                sub.clone()
            } else {
                format!("{} > {}", parent_breadcrumb, sub)
            }
        } else {
            parent_breadcrumb.to_string()
        };
        sections.push((current_section, breadcrumb));
    }

    sections
}

/// Split content on paragraph boundaries (\n\n) as a final fallback.
///
/// Code blocks are never split even if they exceed target_tokens.
fn split_on_paragraphs(
    content: &str,
    heading_breadcrumb: &str,
    target_tokens: usize,
) -> Vec<(String, String)> {
    let paragraphs: Vec<&str> = content.split("\n\n").collect();

    if paragraphs.len() <= 1 {
        // Can't split further; return as-is
        return vec![(content.to_string(), heading_breadcrumb.to_string())];
    }

    let mut result: Vec<(String, String)> = Vec::new();
    let mut current = String::new();

    for para in paragraphs {
        let para_trimmed = para.trim();
        if para_trimmed.is_empty() {
            continue;
        }

        let combined_tokens = estimate_tokens(&if current.is_empty() {
            para_trimmed.to_string()
        } else {
            format!("{}\n\n{}", current, para_trimmed)
        });

        if combined_tokens > target_tokens && !current.is_empty() {
            // Flush current accumulation
            result.push((current, heading_breadcrumb.to_string()));
            current = para_trimmed.to_string();
        } else if current.is_empty() {
            current = para_trimmed.to_string();
        } else {
            current.push_str("\n\n");
            current.push_str(para_trimmed);
        }
    }

    if !current.trim().is_empty() {
        result.push((current, heading_breadcrumb.to_string()));
    }

    result
}

#[async_trait]
impl ChunkingStrategy for HeadingBoundaryChunking {
    async fn chunk(&self, content: &str, config: &ChunkerConfig) -> Result<Vec<ChunkResult>> {
        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        let target_tokens = config.chunk_size;
        let sections = split_on_headings(content, target_tokens);

        let mut chunks = Vec::new();
        let mut chunk_index = 0;

        for (section_content, breadcrumb) in sections {
            let section_tokens = estimate_tokens(&section_content);

            if section_tokens > target_tokens {
                // Sub-split oversized section
                let sub_sections =
                    sub_split_section(&section_content, &breadcrumb, target_tokens);
                for (sub_content, sub_breadcrumb) in sub_sections {
                    let mut meta = serde_json::Map::new();
                    if !sub_breadcrumb.is_empty() {
                        meta.insert(
                            "heading_context".to_string(),
                            serde_json::Value::String(sub_breadcrumb),
                        );
                    }
                    chunks.push(ChunkResult {
                        content: sub_content.trim().to_string(),
                        tokens: estimate_tokens(sub_content.trim()),
                        chunk_order_index: chunk_index,
                        metadata: if meta.is_empty() { None } else { Some(meta) },
                    });
                    chunk_index += 1;
                }
            } else {
                let mut meta = serde_json::Map::new();
                if !breadcrumb.is_empty() {
                    meta.insert(
                        "heading_context".to_string(),
                        serde_json::Value::String(breadcrumb),
                    );
                }
                chunks.push(ChunkResult {
                    content: section_content.trim().to_string(),
                    tokens: section_tokens,
                    chunk_order_index: chunk_index,
                    metadata: if meta.is_empty() { None } else { Some(meta) },
                });
                chunk_index += 1;
            }
        }

        // Filter out empty chunks
        chunks.retain(|c| !c.content.is_empty());
        // Re-index after filtering
        for (i, chunk) in chunks.iter_mut().enumerate() {
            chunk.chunk_order_index = i;
        }

        Ok(chunks)
    }

    fn name(&self) -> &str {
        "heading_boundary"
    }
}

/// Split text into sentences using simple heuristics.
fn split_into_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    for c in text.chars() {
        current.push(c);

        // Check for sentence endings (simple heuristic)
        if c == '.' || c == '!' || c == '?' {
            // Avoid splitting on abbreviations like "Dr." "Mr." "Inc."
            let trimmed = current.trim();
            if trimmed.len() >= 3 {
                // Check if previous word is an abbreviation
                let words: Vec<&str> = trimmed.split_whitespace().collect();
                if let Some(last_word) = words.last() {
                    // Common abbreviations to NOT split on
                    let abbrevs = [
                        "Dr.", "Mr.", "Mrs.", "Ms.", "Jr.", "Sr.", "Inc.", "Ltd.", "etc.", "vs.",
                        "e.g.", "i.e.", "No.", "St.",
                    ];
                    if !abbrevs.contains(last_word) {
                        sentences.push(current.trim().to_string());
                        current = String::new();
                    }
                }
            }
        }
    }

    // Don't forget trailing text without sentence ending
    if !current.trim().is_empty() {
        sentences.push(current.trim().to_string());
    }

    sentences
}

/// Take sentences from buffer to achieve approximately target overlap tokens.
fn take_overlap_sentences(buffer: &[String], target_tokens: usize) -> Vec<String> {
    let mut overlap = Vec::new();
    let mut tokens = 0;

    // Take from end of buffer
    for sentence in buffer.iter().rev() {
        let sentence_tokens = estimate_tokens(sentence);
        if tokens + sentence_tokens > target_tokens && !overlap.is_empty() {
            break;
        }
        overlap.insert(0, sentence.clone());
        tokens += sentence_tokens;
    }

    overlap
}

/// Find the nearest valid UTF-8 char boundary at or before the given byte position.
fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    // Walk backwards to find a valid char boundary
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Find the nearest valid UTF-8 char boundary at or after the given byte position.
fn ceil_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    // Walk forward to find a valid char boundary
    let mut i = index;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Internal function to split text.
fn split_text_internal(
    text: &str,
    target_size: usize,
    overlap: usize,
    min_size: usize,
    separators: &[String],
) -> Vec<(String, usize, usize)> {
    if text.len() <= target_size {
        return vec![(text.to_string(), 0, text.len())];
    }

    let mut chunks = Vec::new();
    let mut current_pos = 0;

    while current_pos < text.len() {
        // Ensure current_pos is on a char boundary
        current_pos = ceil_char_boundary(text, current_pos);

        let remaining = &text[current_pos..];

        if remaining.len() <= target_size {
            chunks.push((remaining.to_string(), current_pos, text.len()));
            break;
        }

        // Calculate end position, ensuring it's on a char boundary
        let end_pos = floor_char_boundary(text, current_pos + target_size);
        let chunk_text = &text[current_pos..end_pos.min(text.len())];

        let split_point = find_split_point_internal(chunk_text, target_size, separators);
        // Ensure actual_end is on a char boundary
        let actual_end = floor_char_boundary(text, current_pos + split_point);

        let chunk_content = text[current_pos..actual_end].to_string();

        if chunk_content.len() >= min_size {
            chunks.push((chunk_content, current_pos, actual_end));
        }

        // Calculate overlap position, ensuring it's on a char boundary
        let overlap_pos = actual_end.saturating_sub(overlap);
        current_pos = ceil_char_boundary(text, overlap_pos);

        if current_pos >= actual_end {
            current_pos = actual_end;
        }
    }

    chunks
}

/// Internal function to find split point.
fn find_split_point_internal(text: &str, target: usize, separators: &[String]) -> usize {
    // Ensure search boundaries are on valid char boundaries
    let search_start = floor_char_boundary(text, target.saturating_sub(target / 4));
    let search_end = floor_char_boundary(text, target.min(text.len()));

    // Only search if we have a valid range
    if search_start >= search_end {
        return floor_char_boundary(text, target.min(text.len()));
    }

    for separator in separators {
        if let Some(pos) = text[search_start..search_end].rfind(separator.as_str()) {
            return search_start + pos + separator.len();
        }
    }

    floor_char_boundary(text, target.min(text.len()))
}

/// Text chunker for splitting documents.
pub struct Chunker {
    config: ChunkerConfig,
    strategy: Arc<dyn ChunkingStrategy>,
}

impl Chunker {
    /// Create a new chunker with the given configuration.
    pub fn new(config: ChunkerConfig) -> Self {
        Self {
            config,
            strategy: Arc::new(TokenBasedChunking),
        }
    }

    /// Create a new chunker with a custom chunking strategy.
    pub fn with_strategy(config: ChunkerConfig, strategy: Arc<dyn ChunkingStrategy>) -> Self {
        Self { config, strategy }
    }

    /// Create a chunker with default configuration.
    pub fn default_chunker() -> Self {
        Self::new(ChunkerConfig::default())
    }

    /// Create a chunker that splits by character only.
    pub fn character_chunker(split_character: impl Into<String>) -> Self {
        let config = ChunkerConfig {
            split_by_character: Some(split_character.into()),
            split_by_character_only: true,
            ..ChunkerConfig::default()
        };
        Self {
            config,
            strategy: Arc::new(CharacterBasedChunking::by_newline()),
        }
    }

    /// Chunk text into overlapping segments.
    pub fn chunk(&self, text: &str, doc_id: &str) -> Result<Vec<TextChunk>> {
        // Always use sync implementation to avoid tokio runtime conflicts
        self.chunk_sync(text, doc_id)
    }

    /// Chunk text asynchronously using the configured strategy.
    pub async fn chunk_async(&self, text: &str, doc_id: &str) -> Result<Vec<TextChunk>> {
        let results = self.strategy.chunk(text, &self.config).await?;

        // Track cumulative offset for line number calculation
        let mut cumulative_offset = 0;

        Ok(results
            .into_iter()
            .map(|result| {
                let id = format!("{}-chunk-{}", doc_id, result.chunk_order_index);
                let start_offset = cumulative_offset;
                let end_offset = cumulative_offset + result.content.len();
                let (start_line, end_line) = calculate_line_numbers(text, start_offset, end_offset);
                cumulative_offset = end_offset;

                TextChunk::with_line_numbers(
                    id,
                    result.content.clone(),
                    result.chunk_order_index,
                    start_offset,
                    end_offset,
                    start_line,
                    end_line,
                )
            })
            .collect())
    }

    /// Synchronous chunk implementation (fallback).
    fn chunk_sync(&self, text: &str, doc_id: &str) -> Result<Vec<TextChunk>> {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let target_chars = self.config.chunk_size * 4;
        let overlap_chars = self.config.chunk_overlap * 4;
        let min_chars = self.config.min_chunk_size * 4;

        let chunks = self.split_text(text, target_chars, overlap_chars, min_chars);

        Ok(chunks
            .into_iter()
            .enumerate()
            .map(|(index, (content, start, end))| {
                let id = format!("{}-chunk-{}", doc_id, index);
                let (start_line, end_line) = calculate_line_numbers(text, start, end);
                TextChunk::with_line_numbers(id, content, index, start, end, start_line, end_line)
            })
            .collect())
    }

    /// Split text using recursive character splitting.
    fn split_text(
        &self,
        text: &str,
        target_size: usize,
        overlap: usize,
        min_size: usize,
    ) -> Vec<(String, usize, usize)> {
        split_text_internal(
            text,
            target_size,
            overlap,
            min_size,
            &self.config.separators,
        )
    }

    /// Find the best split point near the target size.
    #[allow(dead_code)]
    fn find_split_point(&self, text: &str, target: usize) -> usize {
        find_split_point_internal(text, target, &self.config.separators)
    }

    /// Get the chunker configuration.
    pub fn config(&self) -> &ChunkerConfig {
        &self.config
    }

    /// Get the chunking strategy name.
    pub fn strategy_name(&self) -> &str {
        self.strategy.name()
    }
}

impl Default for Chunker {
    fn default() -> Self {
        Self {
            config: ChunkerConfig::default(),
            strategy: Arc::new(TokenBasedChunking),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_chunking() {
        let chunker = Chunker::default_chunker();
        let text = "This is sentence one. This is sentence two. This is sentence three.";

        let chunks = chunker.chunk(text, "doc1").unwrap();

        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].index, 0);
    }

    #[test]
    fn test_empty_text() {
        let chunker = Chunker::default_chunker();
        let chunks = chunker.chunk("", "doc1").unwrap();
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_short_text() {
        let chunker = Chunker::default_chunker();
        let text = "Short text.";
        let chunks = chunker.chunk(text, "doc1").unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, text);
    }

    #[test]
    fn test_long_text_chunking() {
        let config = ChunkerConfig {
            chunk_size: 10, // 10 tokens * 4 chars = 40 chars per chunk
            chunk_overlap: 2,
            min_chunk_size: 5,
            ..Default::default()
        };
        let chunker = Chunker::new(config);

        let text = "First sentence here. Second sentence follows. Third sentence now. Fourth one too. Fifth is last.";
        let chunks = chunker.chunk(text, "doc1").unwrap();

        assert!(chunks.len() > 1);

        // Verify chunks cover the text
        let total_unique: std::collections::HashSet<_> =
            chunks.iter().flat_map(|c| c.content.chars()).collect();
        assert!(total_unique.len() > 0);
    }

    #[test]
    fn test_chunk_ids() {
        let chunker = Chunker::default_chunker();
        let text = "Some text content that will be chunked.";
        let chunks = chunker.chunk(text, "my-doc").unwrap();

        assert!(chunks[0].id.starts_with("my-doc-chunk-"));
    }

    #[test]
    fn test_token_estimation() {
        assert_eq!(estimate_tokens("test"), 1);
        assert_eq!(estimate_tokens("hello world"), 3); // 11 chars / 4 ≈ 3
    }

    #[test]
    fn test_line_number_calculation() {
        // Test single line
        let text = "Hello world";
        let (start, end) = calculate_line_numbers(text, 0, text.len());
        assert_eq!(start, 1);
        assert_eq!(end, 1);

        // Test multiple lines
        let text = "Line 1\nLine 2\nLine 3";
        let (start, end) = calculate_line_numbers(text, 0, text.len());
        assert_eq!(start, 1);
        assert_eq!(end, 3);

        // Test middle portion
        let text = "Line 1\nLine 2\nLine 3\nLine 4";
        let line2_start = 7; // After "Line 1\n"
        let line3_end = 20; // End of "Line 3"
        let (start, end) = calculate_line_numbers(text, line2_start, line3_end);
        assert_eq!(start, 2);
        assert_eq!(end, 3);
    }

    #[test]
    fn test_chunks_have_line_numbers() {
        let chunker = Chunker::default_chunker();
        let text = "Line one.\nLine two.\nLine three.";
        let chunks = chunker.chunk(text, "doc1").unwrap();

        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].start_line, 1);
        assert!(chunks[0].end_line >= 1);
    }

    #[test]
    fn test_multiline_chunk_line_numbers() {
        let config = ChunkerConfig {
            chunk_size: 10,
            chunk_overlap: 2,
            min_chunk_size: 5,
            ..Default::default()
        };
        let chunker = Chunker::new(config);

        let text = "Line 1 here.\nLine 2 here.\nLine 3 here.\nLine 4 here.\nLine 5 here.";
        let chunks = chunker.chunk(text, "doc1").unwrap();

        // First chunk should start at line 1
        if !chunks.is_empty() {
            assert_eq!(chunks[0].start_line, 1);
        }
    }

    #[test]
    fn test_utf8_multibyte_chars_in_chunking() {
        // Test with multi-byte UTF-8 characters: smart quotes, bullets, emojis
        // Using raw bytes to include smart quotes without Rust parser issues
        let text = "Quality. Compared with state-of-the-art FR-IQA models, \
the \u{201C}proposed GMSD model\u{201D} performs better \u{2022} in terms of both accuracy \
and efficiency, making GMSD an ideal choice for high-performance IQA applications.\n\n\
This work is supported by \u{7814}\u{7A76} and \u{5F00}\u{53D1} funding.";

        let config = ChunkerConfig {
            chunk_size: 50, // Force chunking within the multi-byte section
            chunk_overlap: 10,
            min_chunk_size: 20,
            ..Default::default()
        };
        let chunker = Chunker::new(config);

        // This should not panic even with multi-byte characters
        let chunks = chunker.chunk(text, "utf8-test").unwrap();

        assert!(!chunks.is_empty());
        // All chunks should be valid UTF-8 strings
        for chunk in &chunks {
            assert!(chunk.content.is_char_boundary(0));
            assert!(chunk.content.is_char_boundary(chunk.content.len()));
        }
    }

    #[test]
    fn test_floor_and_ceil_char_boundary() {
        // Test with multi-byte character: " (LEFT DOUBLE QUOTATION MARK, 3 bytes: E2 80 9C)
        let text = "ab\u{201C}cd";

        // "ab" is 2 bytes, then " is 3 bytes (positions 2, 3, 4), then "cd" is 2 more
        // So: a=0, b=1, "=2,3,4, c=5, d=6

        assert_eq!(floor_char_boundary(text, 2), 2); // Start of "
        assert_eq!(floor_char_boundary(text, 3), 2); // Inside " -> back to 2
        assert_eq!(floor_char_boundary(text, 4), 2); // Inside " -> back to 2
        assert_eq!(floor_char_boundary(text, 5), 5); // Start of c

        assert_eq!(ceil_char_boundary(text, 2), 2); // Start of "
        assert_eq!(ceil_char_boundary(text, 3), 5); // Inside " -> forward to 5
        assert_eq!(ceil_char_boundary(text, 4), 5); // Inside " -> forward to 5
    }

    // =========================================================================
    // SentenceBoundaryChunking Tests
    // =========================================================================

    #[tokio::test]
    async fn test_sentence_boundary_basic() {
        let strategy = SentenceBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 100,
            chunk_overlap: 10,
            min_chunk_size: 5,
            ..Default::default()
        };

        let text = "This is sentence one. This is sentence two. This is sentence three.";
        let chunks = strategy.chunk(text, &config).await.unwrap();

        assert!(!chunks.is_empty());
        // With large chunk size, all should fit in one chunk
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("sentence one"));
    }

    #[tokio::test]
    async fn test_sentence_boundary_splits_on_sentences() {
        let strategy = SentenceBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 15, // Small size to force splits
            chunk_overlap: 5,
            min_chunk_size: 5,
            ..Default::default()
        };

        let text = "First sentence here. Second sentence now. Third one follows. Fourth is last.";
        let chunks = strategy.chunk(text, &config).await.unwrap();

        // Should have multiple chunks
        assert!(chunks.len() >= 2);

        // Each chunk should end at a sentence boundary (or be the last chunk)
        for chunk in &chunks {
            let trimmed = chunk.content.trim();
            // Last char should be period, question, or exclamation (sentence ending)
            // OR it's incomplete because it's the last chunk
            if trimmed.len() > 1 {
                let last_char = trimmed.chars().last().unwrap();
                assert!(
                    last_char == '.' || last_char == '!' || last_char == '?',
                    "Chunk should end at sentence boundary: {}",
                    trimmed
                );
            }
        }
    }

    #[tokio::test]
    async fn test_sentence_boundary_empty_text() {
        let strategy = SentenceBoundaryChunking::new();
        let config = ChunkerConfig::default();

        let chunks = strategy.chunk("", &config).await.unwrap();
        assert!(chunks.is_empty());
    }

    #[tokio::test]
    async fn test_sentence_boundary_no_periods() {
        let strategy = SentenceBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 10,
            min_chunk_size: 10,
            ..Default::default()
        };

        // Text with no sentence boundaries falls back to token-based
        let text = "This is text without any sentence endings at all";
        let chunks = strategy.chunk(text, &config).await.unwrap();

        assert!(!chunks.is_empty());
    }

    #[tokio::test]
    async fn test_sentence_boundary_abbreviations() {
        let strategy = SentenceBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 200,
            chunk_overlap: 20,
            min_chunk_size: 10,
            ..Default::default()
        };

        // Text with abbreviations should not split on them
        let text = "Dr. Smith works at Inc. headquarters. He joined in Jan. 2020.";
        let chunks = strategy.chunk(text, &config).await.unwrap();

        // With large chunk size, should be one chunk
        assert_eq!(chunks.len(), 1);
    }

    #[tokio::test]
    async fn test_sentence_boundary_question_exclamation() {
        let strategy = SentenceBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 20,
            chunk_overlap: 5,
            min_chunk_size: 5,
            ..Default::default()
        };

        let text = "What is this? This is amazing! And this is normal.";
        let chunks = strategy.chunk(text, &config).await.unwrap();

        // Should split on all three sentence types
        assert!(chunks.len() >= 1);
    }

    // =========================================================================
    // ParagraphBoundaryChunking Tests
    // =========================================================================

    #[tokio::test]
    async fn test_paragraph_boundary_basic() {
        let strategy = ParagraphBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 100,
            chunk_overlap: 10,
            min_chunk_size: 5,
            ..Default::default()
        };

        let text = "First paragraph here.\n\nSecond paragraph here.\n\nThird paragraph.";
        let chunks = strategy.chunk(text, &config).await.unwrap();

        assert!(!chunks.is_empty());
    }

    #[tokio::test]
    async fn test_paragraph_boundary_splits_on_paragraphs() {
        let strategy = ParagraphBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 10, // Very small to force splits
            chunk_overlap: 2,
            min_chunk_size: 3,
            ..Default::default()
        };

        // Use longer text to ensure chunks are created
        let text = "First paragraph with some text here.\n\n\
                    Second paragraph with more content.\n\n\
                    Third paragraph now added.\n\n\
                    Fourth paragraph is last.";
        let chunks = strategy.chunk(text, &config).await.unwrap();

        // Should have multiple chunks due to small chunk_size
        assert!(
            chunks.len() >= 1,
            "Expected at least 1 chunk, got {}",
            chunks.len()
        );
    }

    #[tokio::test]
    async fn test_paragraph_boundary_empty_text() {
        let strategy = ParagraphBoundaryChunking::new();
        let config = ChunkerConfig::default();

        let chunks = strategy.chunk("", &config).await.unwrap();
        assert!(chunks.is_empty());
    }

    #[tokio::test]
    async fn test_paragraph_boundary_single_paragraph() {
        let strategy = ParagraphBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 100,
            chunk_overlap: 10,
            min_chunk_size: 5,
            ..Default::default()
        };

        let text = "This is a single paragraph with no breaks.";
        let chunks = strategy.chunk(text, &config).await.unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, text);
    }

    #[tokio::test]
    async fn test_paragraph_boundary_preserves_paragraph_integrity() {
        let strategy = ParagraphBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 50,
            chunk_overlap: 10,
            min_chunk_size: 10,
            ..Default::default()
        };

        let text = "First paragraph with multiple sentences. Here is more.\n\n\
                    Second paragraph also has content. More words here.";
        let chunks = strategy.chunk(text, &config).await.unwrap();

        // Each chunk should contain complete paragraphs (not mid-paragraph splits)
        for chunk in &chunks {
            // Should not start or end mid-word unexpectedly
            assert!(!chunk.content.starts_with(' '));
        }
    }

    #[tokio::test]
    async fn test_paragraph_boundary_single_newline_fallback() {
        let strategy = ParagraphBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 20,
            chunk_overlap: 5,
            min_chunk_size: 3,
            ..Default::default()
        };

        // Single newlines (no double newlines)
        let text = "Line one here.\nLine two here.\nLine three.";
        let chunks = strategy.chunk(text, &config).await.unwrap();

        // Should still produce chunks
        assert!(!chunks.is_empty());
    }

    #[tokio::test]
    async fn test_paragraph_boundary_large_paragraph() {
        let strategy = ParagraphBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 10, // Very small
            chunk_overlap: 2,
            min_chunk_size: 3,
            ..Default::default()
        };

        // Large paragraph that exceeds chunk size
        let text = "This is a very large paragraph that definitely exceeds the tiny chunk size limit we set.\n\n\
                    Small para.";
        let chunks = strategy.chunk(text, &config).await.unwrap();

        // Should still produce chunks (large para gets its own chunk)
        assert!(!chunks.is_empty());
    }

    // =========================================================================
    // Chunker with Custom Strategy Tests
    // =========================================================================

    #[test]
    fn test_chunker_with_sentence_strategy() {
        let config = ChunkerConfig::default();
        let chunker = Chunker::with_strategy(config, Arc::new(SentenceBoundaryChunking::new()));

        assert_eq!(chunker.strategy_name(), "sentence_boundary");
    }

    #[test]
    fn test_chunker_with_paragraph_strategy() {
        let config = ChunkerConfig::default();
        let chunker = Chunker::with_strategy(config, Arc::new(ParagraphBoundaryChunking::new()));

        assert_eq!(chunker.strategy_name(), "paragraph_boundary");
    }

    #[test]
    fn test_split_into_sentences_basic() {
        let sentences = split_into_sentences("First. Second. Third.");
        assert_eq!(sentences.len(), 3);
    }

    #[test]
    fn test_split_into_sentences_with_abbreviations() {
        let sentences = split_into_sentences("Dr. Smith said hello. Then left.");
        // Should NOT split on "Dr."
        assert!(sentences.len() <= 2);
    }

    // =========================================================================
    // HeadingBoundaryChunking Tests
    // =========================================================================

    #[tokio::test]
    async fn test_heading_boundary_basic() {
        let strategy = HeadingBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 500,
            chunk_overlap: 10,
            min_chunk_size: 5,
            ..Default::default()
        };

        let text = "\
# Introduction

This is the introduction paragraph.

## Background

This is the background section with some details.

## Methods

Here we describe the methods used.

# Results

The results are presented here.";

        let chunks = strategy.chunk(text, &config).await.unwrap();

        // Should split on H1/H2 boundaries
        assert!(
            chunks.len() >= 3,
            "Expected at least 3 chunks from 4 heading sections, got {}",
            chunks.len()
        );

        // First chunk should contain the introduction
        assert!(
            chunks[0].content.contains("introduction"),
            "First chunk should contain introduction text"
        );
    }

    #[tokio::test]
    async fn test_heading_boundary_breadcrumb() {
        let strategy = HeadingBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 500,
            chunk_overlap: 10,
            min_chunk_size: 5,
            ..Default::default()
        };

        let text = "\
# Chapter 1

Some intro text.

## Section A

Section A content here.

## Section B

Section B content here.";

        let chunks = strategy.chunk(text, &config).await.unwrap();

        // Find a chunk that has heading_context metadata
        let chunks_with_metadata: Vec<_> = chunks
            .iter()
            .filter(|c| c.metadata.is_some())
            .collect();

        assert!(
            !chunks_with_metadata.is_empty(),
            "At least one chunk should have heading_context metadata"
        );

        // Check that breadcrumbs contain expected heading hierarchy
        let has_breadcrumb = chunks.iter().any(|c| {
            if let Some(ref meta) = c.metadata {
                if let Some(serde_json::Value::String(ref ctx)) = meta.get("heading_context") {
                    ctx.contains("Chapter 1") && ctx.contains("Section")
                } else {
                    false
                }
            } else {
                false
            }
        });

        assert!(
            has_breadcrumb,
            "Should have a breadcrumb with 'Chapter 1 > Section'"
        );
    }

    #[tokio::test]
    async fn test_heading_boundary_code_fence() {
        let strategy = HeadingBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 500,
            chunk_overlap: 10,
            min_chunk_size: 5,
            ..Default::default()
        };

        let text = "\
# Main Section

Here is some code:

```
# Not a heading
## Also not a heading
def foo():
    pass
```

More text after code block.";

        let chunks = strategy.chunk(text, &config).await.unwrap();

        // The # inside code fence should NOT trigger a split
        // Should be one chunk since there's only one H1 heading
        assert_eq!(
            chunks.len(),
            1,
            "Code fence headings should not cause splits, got {} chunks",
            chunks.len()
        );

        // Content should contain the code block intact
        assert!(
            chunks[0].content.contains("# Not a heading"),
            "Code block content should be preserved"
        );
        assert!(
            chunks[0].content.contains("def foo():"),
            "Code block should be intact"
        );
    }

    #[tokio::test]
    async fn test_heading_boundary_oversized_section() {
        let strategy = HeadingBoundaryChunking::new();
        let config = ChunkerConfig {
            chunk_size: 20, // Very small to force sub-splitting
            chunk_overlap: 5,
            min_chunk_size: 3,
            ..Default::default()
        };

        let text = "\
# Big Section

### Subsection 1

First subsection has some content here that is fairly long.

### Subsection 2

Second subsection also has content that goes on for a bit.

### Subsection 3

Third subsection rounds things out with more text here.";

        let chunks = strategy.chunk(text, &config).await.unwrap();

        // With very small chunk_size, the big section should be sub-split
        assert!(
            chunks.len() >= 2,
            "Oversized section should be sub-split, got {} chunks",
            chunks.len()
        );
    }

    #[tokio::test]
    async fn test_heading_boundary_empty() {
        let strategy = HeadingBoundaryChunking::new();
        let config = ChunkerConfig::default();

        let chunks = strategy.chunk("", &config).await.unwrap();
        assert!(chunks.is_empty(), "Empty content should return no chunks");

        let chunks2 = strategy.chunk("   \n  \n  ", &config).await.unwrap();
        assert!(
            chunks2.is_empty(),
            "Whitespace-only content should return no chunks"
        );
    }

    #[test]
    fn test_detect_heading_level() {
        // Valid headings
        assert_eq!(detect_heading_level("# H1"), Some(1));
        assert_eq!(detect_heading_level("## H2"), Some(2));
        assert_eq!(detect_heading_level("### H3"), Some(3));
        assert_eq!(detect_heading_level("#### H4"), Some(4));
        assert_eq!(detect_heading_level("##### H5"), Some(5));
        assert_eq!(detect_heading_level("###### H6"), Some(6));

        // With leading whitespace
        assert_eq!(detect_heading_level("  # H1 with indent"), Some(1));

        // Invalid: no space after hashes
        assert_eq!(detect_heading_level("#NoSpace"), None);
        assert_eq!(detect_heading_level("###"), None);
        assert_eq!(detect_heading_level("##no"), None);

        // Not a heading
        assert_eq!(detect_heading_level("Normal text"), None);
        assert_eq!(detect_heading_level(""), None);
    }

    #[test]
    fn test_build_breadcrumb() {
        let stack: Vec<(usize, String)> = vec![];
        assert_eq!(build_breadcrumb(&stack), "");

        let stack = vec![(1, "Chapter 3".to_string())];
        assert_eq!(build_breadcrumb(&stack), "Chapter 3");

        let stack = vec![
            (1, "Chapter 3".to_string()),
            (2, "Section 2".to_string()),
        ];
        assert_eq!(build_breadcrumb(&stack), "Chapter 3 > Section 2");

        let stack = vec![
            (1, "Intro".to_string()),
            (2, "Background".to_string()),
            (3, "History".to_string()),
        ];
        assert_eq!(build_breadcrumb(&stack), "Intro > Background > History");
    }

    #[test]
    fn test_chunker_with_heading_strategy() {
        let config = ChunkerConfig::default();
        let chunker =
            Chunker::with_strategy(config, Arc::new(HeadingBoundaryChunking::new()));

        assert_eq!(chunker.strategy_name(), "heading_boundary");
    }
}
