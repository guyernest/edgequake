::: exercise
id: markdown-chunking
difficulty: intermediate
time: 25 minutes
:::

# Markdown-Aware Chunking

Raw text cannot be fed to an embedding model in one piece -- it must be split
into chunks that fit the model's context window and carry coherent meaning.
Naive character-based splitting destroys semantic boundaries. In this exercise
you will implement a chunking strategy that respects markdown structure,
splitting on headers while enforcing a maximum chunk size.

::: objectives
thinking:
  - Understand why chunk boundaries affect retrieval quality
  - Reason about the tradeoff between chunk size and semantic coherence
doing:
  - Parse markdown ATX headers (# through ######) to identify section boundaries
  - Split text at header boundaries while enforcing a max_size constraint
  - Preserve header context so each chunk knows which section it belongs to
:::

::: discussion
- Why does splitting mid-sentence hurt retrieval quality compared to splitting at section boundaries?
- What is the ideal chunk size for embedding models, and how does it vary by model?
- How would you handle a markdown document with no headers at all?
:::

::: starter file="src/main.rs"
```rust
/// A chunk of text with metadata about its position in the document.
#[derive(Debug, Clone, PartialEq)]
struct Chunk {
    /// The text content of this chunk.
    content: String,
    /// The header path for this chunk (e.g., "# Introduction > ## Setup").
    header_path: String,
    /// Character offset where this chunk starts in the original document.
    start_offset: usize,
}

/// Configuration for the markdown chunking strategy.
struct ChunkConfig {
    /// Maximum number of characters per chunk.
    max_size: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        ChunkConfig { max_size: 500 }
    }
}

/// Determine the header level of a line (1-6), or 0 if not a header.
///
/// ATX headers start with 1-6 '#' characters followed by a space.
fn header_level(line: &str) -> usize {
    // TODO: Implement header level detection
    // - Count leading '#' characters
    // - Return the count if it is 1-6 AND followed by a space (or end of line)
    // - Return 0 if the line is not a valid ATX header
    todo!("Implement header_level")
}

/// Extract the header text (without the '#' prefix).
fn header_text(line: &str) -> &str {
    // TODO: Strip the leading '#' characters and whitespace
    todo!("Implement header_text")
}

/// Split a markdown document into chunks respecting header boundaries
/// and a maximum chunk size.
///
/// Rules:
/// 1. Each header starts a new chunk.
/// 2. If a section (text between headers) exceeds max_size, split it at
///    paragraph boundaries (blank lines).
/// 3. If a single paragraph exceeds max_size, split at the last space
///    before max_size.
/// 4. Each chunk carries a header_path showing its position in the
///    document hierarchy (e.g., "# Title > ## Section").
fn chunk_markdown(text: &str, config: &ChunkConfig) -> Vec<Chunk> {
    // TODO: Implement markdown chunking
    //
    // Suggested approach:
    // 1. Split the text into lines.
    // 2. Walk through lines, tracking the current header stack.
    // 3. When you hit a new header, flush the current chunk and update
    //    the header stack.
    // 4. After collecting all section text, enforce max_size by splitting
    //    oversized sections at paragraph boundaries.
    // 5. Build the header_path from the header stack.
    todo!("Implement chunk_markdown")
}

/// Build a header path string from a stack of headers.
/// Example: ["# Introduction", "## Setup"] -> "# Introduction > ## Setup"
fn build_header_path(stack: &[String]) -> String {
    stack.join(" > ")
}

/// Split text into sub-chunks at paragraph boundaries, respecting max_size.
fn split_oversized(text: &str, max_size: usize) -> Vec<String> {
    // TODO: Split at paragraph boundaries ("\n\n") first.
    // If a single paragraph is still too large, split at the last space
    // before max_size.
    todo!("Implement split_oversized")
}

fn main() {
    let markdown = r#"# Graph RAG Overview

Graph RAG combines knowledge graphs with retrieval-augmented generation
to provide more accurate and contextual answers.

## How It Works

Documents are processed through an ingestion pipeline that extracts
entities and relationships, building a knowledge graph.

### Extraction

Named entity recognition and relationship extraction identify the
key concepts and how they connect.

### Embedding

Each chunk is embedded into a vector space for similarity search.

## Query Processing

When a query arrives, the system selects the appropriate retrieval
mode based on the query characteristics.
"#;

    let config = ChunkConfig { max_size: 200 };
    let chunks = chunk_markdown(markdown, &config);

    for (i, chunk) in chunks.iter().enumerate() {
        println!("--- Chunk {} ---", i);
        println!("Path: {}", chunk.header_path);
        println!("Content: {}...", &chunk.content[..chunk.content.len().min(80)]);
        println!();
    }
}
```
:::

::: hint level=1 title="Header detection"
A valid ATX header line starts with 1-6 `#` characters, followed by a space
or end-of-line. Use `chars()` to count leading hashes:

```rust
fn header_level(line: &str) -> usize {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if hashes >= 1 && hashes <= 6 {
        // Must be followed by a space or be the entire line
        if trimmed.len() == hashes || trimmed.as_bytes()[hashes] == b' ' {
            return hashes;
        }
    }
    0
}
```
:::

::: hint level=2 title="Chunking algorithm outline"
Maintain a `header_stack: Vec<String>` and a `current_section: String`. Walk
lines. On each header: flush current_section as a chunk (if non-empty), update
the header_stack (pop headers at equal or deeper level, push the new one), and
start a new current_section. At end-of-input, flush the final section.

For max_size enforcement, split each section's content using `split_oversized`
before creating chunks.
:::

::: solution
```rust
#[derive(Debug, Clone, PartialEq)]
struct Chunk {
    content: String,
    header_path: String,
    start_offset: usize,
}

struct ChunkConfig {
    max_size: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        ChunkConfig { max_size: 500 }
    }
}

fn header_level(line: &str) -> usize {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if hashes >= 1 && hashes <= 6 {
        if trimmed.len() == hashes || trimmed.as_bytes()[hashes] == b' ' {
            return hashes;
        }
    }
    0
}

fn header_text(line: &str) -> &str {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    trimmed[hashes..].trim()
}

fn build_header_path(stack: &[String]) -> String {
    stack.join(" > ")
}

fn split_oversized(text: &str, max_size: usize) -> Vec<String> {
    if text.len() <= max_size {
        return vec![text.to_string()];
    }

    let mut results = Vec::new();
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let mut current = String::new();

    for para in paragraphs {
        if current.is_empty() {
            if para.len() > max_size {
                // Split long paragraph at word boundaries
                let mut remaining = para;
                while remaining.len() > max_size {
                    let split_at = remaining[..max_size]
                        .rfind(' ')
                        .unwrap_or(max_size);
                    results.push(remaining[..split_at].trim().to_string());
                    remaining = remaining[split_at..].trim_start();
                }
                if !remaining.is_empty() {
                    current = remaining.to_string();
                }
            } else {
                current = para.to_string();
            }
        } else if current.len() + 2 + para.len() > max_size {
            results.push(current.clone());
            if para.len() > max_size {
                let mut remaining = para;
                while remaining.len() > max_size {
                    let split_at = remaining[..max_size]
                        .rfind(' ')
                        .unwrap_or(max_size);
                    results.push(remaining[..split_at].trim().to_string());
                    remaining = remaining[split_at..].trim_start();
                }
                current = remaining.to_string();
            } else {
                current = para.to_string();
            }
        } else {
            current.push_str("\n\n");
            current.push_str(para);
        }
    }

    if !current.trim().is_empty() {
        results.push(current);
    }

    results
}

fn chunk_markdown(text: &str, config: &ChunkConfig) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut header_stack: Vec<(usize, String)> = Vec::new(); // (level, "## Text")
    let mut current_content = String::new();
    let mut current_offset: usize = 0;
    let mut offset: usize = 0;

    let lines: Vec<&str> = text.lines().collect();

    for line in &lines {
        let level = header_level(line);

        if level > 0 {
            // Flush current content
            let trimmed = current_content.trim().to_string();
            if !trimmed.is_empty() {
                let path = build_header_path(
                    &header_stack.iter().map(|(_, h)| h.clone()).collect::<Vec<_>>(),
                );
                let sub_chunks = split_oversized(&trimmed, config.max_size);
                for sub in sub_chunks {
                    chunks.push(Chunk {
                        content: sub,
                        header_path: path.clone(),
                        start_offset: current_offset,
                    });
                }
            }

            // Update header stack: pop anything at same or deeper level
            while header_stack.last().map_or(false, |(l, _)| *l >= level) {
                header_stack.pop();
            }
            let header_label = format!(
                "{} {}",
                "#".repeat(level),
                header_text(line)
            );
            header_stack.push((level, header_label));
            current_content = String::new();
            current_offset = offset + line.len() + 1; // +1 for newline
        } else {
            if !current_content.is_empty() || !line.is_empty() {
                if !current_content.is_empty() {
                    current_content.push('\n');
                }
                current_content.push_str(line);
            }
        }
        offset += line.len() + 1;
    }

    // Flush final content
    let trimmed = current_content.trim().to_string();
    if !trimmed.is_empty() {
        let path = build_header_path(
            &header_stack.iter().map(|(_, h)| h.clone()).collect::<Vec<_>>(),
        );
        let sub_chunks = split_oversized(&trimmed, config.max_size);
        for sub in sub_chunks {
            chunks.push(Chunk {
                content: sub,
                header_path: path.clone(),
                start_offset: current_offset,
            });
        }
    }

    chunks
}

fn main() {
    let markdown = r#"# Graph RAG Overview

Graph RAG combines knowledge graphs with retrieval-augmented generation
to provide more accurate and contextual answers.

## How It Works

Documents are processed through an ingestion pipeline that extracts
entities and relationships, building a knowledge graph.

### Extraction

Named entity recognition and relationship extraction identify the
key concepts and how they connect.

### Embedding

Each chunk is embedded into a vector space for similarity search.

## Query Processing

When a query arrives, the system selects the appropriate retrieval
mode based on the query characteristics.
"#;

    let config = ChunkConfig { max_size: 200 };
    let chunks = chunk_markdown(markdown, &config);

    for (i, chunk) in chunks.iter().enumerate() {
        println!("--- Chunk {} ---", i);
        println!("Path: {}", chunk.header_path);
        println!("Content: {}...", &chunk.content[..chunk.content.len().min(80)]);
        println!();
    }
}
```

### Explanation

The chunker walks through lines, detecting ATX headers to determine section
boundaries. A header stack tracks the current position in the document
hierarchy, popping headers at the same or deeper level when a new header is
encountered. This ensures the header_path correctly reflects nesting (e.g.,
"# Title > ## Section > ### Subsection").

When a section is flushed, `split_oversized` enforces the max_size constraint
by first trying paragraph boundaries (double newlines), then falling back to
word boundaries (last space before max_size). This preserves readability while
respecting size limits.
:::

::: tests mode=local
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_level_h1() {
        assert_eq!(header_level("# Title"), 1);
    }

    #[test]
    fn test_header_level_h3() {
        assert_eq!(header_level("### Subsection"), 3);
    }

    #[test]
    fn test_header_level_not_a_header() {
        assert_eq!(header_level("This is not a header"), 0);
    }

    #[test]
    fn test_header_level_no_space_after_hash() {
        assert_eq!(header_level("#NoSpace"), 0);
    }

    #[test]
    fn test_header_text_extraction() {
        assert_eq!(header_text("## Hello World"), "Hello World");
    }

    #[test]
    fn test_basic_chunking() {
        let md = "# A\n\nFirst section.\n\n# B\n\nSecond section.\n";
        let config = ChunkConfig { max_size: 500 };
        let chunks = chunk_markdown(md, &config);

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].header_path, "# A");
        assert!(chunks[0].content.contains("First section"));
        assert_eq!(chunks[1].header_path, "# B");
        assert!(chunks[1].content.contains("Second section"));
    }

    #[test]
    fn test_nested_headers() {
        let md = "# Top\n\n## Sub\n\nContent here.\n";
        let config = ChunkConfig { max_size: 500 };
        let chunks = chunk_markdown(md, &config);

        // The content "Content here." lives under ## Sub
        let content_chunk = chunks.iter().find(|c| c.content.contains("Content here")).unwrap();
        assert!(content_chunk.header_path.contains("# Top"));
        assert!(content_chunk.header_path.contains("## Sub"));
    }

    #[test]
    fn test_max_size_enforcement() {
        let long_section = format!("# Title\n\n{}\n\n{}\n", "A".repeat(100), "B".repeat(100));
        let config = ChunkConfig { max_size: 120 };
        let chunks = chunk_markdown(&long_section, &config);

        for chunk in &chunks {
            assert!(
                chunk.content.len() <= 120,
                "Chunk exceeded max_size: {} chars",
                chunk.content.len()
            );
        }
    }

    #[test]
    fn test_empty_document() {
        let chunks = chunk_markdown("", &ChunkConfig::default());
        assert!(chunks.is_empty(), "Empty document should produce no chunks");
    }

    #[test]
    fn test_split_oversized_respects_paragraphs() {
        let text = "Paragraph one.\n\nParagraph two.\n\nParagraph three.";
        let parts = split_oversized(text, 30);
        assert!(parts.len() >= 2, "Should split into multiple parts");
        for part in &parts {
            assert!(part.len() <= 30, "Part exceeded max_size: {}", part.len());
        }
    }
}
```
:::

::: reflection
- How would you add chunk overlap (e.g., repeating the last N characters of each chunk at the start of the next) to improve retrieval at boundaries?
- What metadata beyond the header path would be useful to attach to each chunk?
- How does the choice of max_size affect downstream embedding quality and retrieval precision?
:::
