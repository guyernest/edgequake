::: exercise
id: entity-extraction
difficulty: intermediate
time: 25 minutes
:::

# Entity Extraction Prompt and Parser

In the Graph RAG ingestion pipeline, raw text must be transformed into
structured knowledge: entities (nodes) and relationships (edges). This is done
by sending the text to an LLM with a carefully crafted prompt that instructs
it to output structured JSON. In this exercise you will build the extraction
prompt template and a robust parser that handles the messy reality of LLM
output.

::: objectives
thinking:
  - Understand how prompt engineering controls LLM output structure
  - Reason about why LLM output parsing must be defensive
doing:
  - Complete a prompt template that elicits structured entity/relationship JSON
  - Implement a JSON parser that handles common LLM output quirks
  - Deduplicate extracted entities by normalized name
:::

::: discussion
- Why do few-shot examples in the prompt improve extraction quality over zero-shot?
- What are the most common ways an LLM deviates from the requested output format?
- How would you handle entity types that the prompt did not anticipate?
:::

::: starter file="src/main.rs"
```rust
use std::collections::HashMap;

/// An extracted entity from text.
#[derive(Debug, Clone, PartialEq)]
struct Entity {
    name: String,
    entity_type: String,
    description: String,
}

/// An extracted relationship between two entities.
#[derive(Debug, Clone, PartialEq)]
struct Relationship {
    source: String,
    target: String,
    relation_type: String,
    description: String,
}

/// The result of entity extraction from a chunk of text.
#[derive(Debug, Clone)]
struct ExtractionResult {
    entities: Vec<Entity>,
    relationships: Vec<Relationship>,
}

/// Build the extraction prompt for a given text chunk.
///
/// The prompt should instruct the LLM to:
/// 1. Extract all named entities with their type and a brief description.
/// 2. Extract all relationships between entities.
/// 3. Output valid JSON matching the expected schema.
fn build_extraction_prompt(text: &str) -> String {
    // TODO: Complete this prompt template.
    //
    // Requirements:
    // - Include the system instruction explaining the task.
    // - Specify the exact JSON schema expected.
    // - Include one few-shot example showing correct output.
    // - Include the actual text to extract from.
    //
    // Expected JSON schema:
    // {
    //   "entities": [
    //     {"name": "...", "type": "...", "description": "..."}
    //   ],
    //   "relationships": [
    //     {"source": "...", "target": "...", "type": "...", "description": "..."}
    //   ]
    // }
    todo!("Implement build_extraction_prompt")
}

/// Parse the LLM response into an ExtractionResult.
///
/// Must handle common LLM quirks:
/// - Response wrapped in markdown code fences (```json ... ```)
/// - Leading/trailing whitespace
/// - Missing optional fields (default to empty string)
fn parse_extraction_response(response: &str) -> Result<ExtractionResult, String> {
    // TODO: Implement the parser.
    //
    // Steps:
    // 1. Strip markdown code fences if present.
    // 2. Find the JSON object boundaries ({ ... }).
    // 3. Parse the JSON manually (no serde -- use simple string parsing).
    // 4. Extract entities and relationships arrays.
    // 5. Deduplicate entities by normalized name (lowercase, trimmed).
    //
    // Hint: Since we cannot use serde in this exercise, implement a
    // minimal JSON parser that handles the specific schema we expect.
    todo!("Implement parse_extraction_response")
}

/// Strip markdown code fences from a string.
/// Handles ```json\n...\n``` and ```\n...\n``` variants.
fn strip_code_fences(text: &str) -> &str {
    // TODO: Implement code fence stripping
    todo!("Implement strip_code_fences")
}

/// Simple JSON string value extractor.
/// Given a key name, extracts the string value from a JSON-like line.
/// Example: extract_json_string_value("\"name\": \"Alice\"", "name") -> Some("Alice")
fn extract_json_string_value(line: &str, key: &str) -> Option<String> {
    // TODO: Implement a simple key-value extractor for JSON strings
    todo!("Implement extract_json_string_value")
}

/// Deduplicate entities by normalized name (lowercase, trimmed).
/// When duplicates are found, keep the one with the longer description.
fn deduplicate_entities(entities: Vec<Entity>) -> Vec<Entity> {
    // TODO: Implement deduplication
    todo!("Implement deduplicate_entities")
}

fn main() {
    let text = r#"
Alice Johnson is the CEO of TechCorp, a software company based in Seattle.
She previously worked at DataInc where she met Bob Smith, who is now the
CTO of TechCorp. TechCorp recently acquired CloudBase for $50 million.
"#;

    let prompt = build_extraction_prompt(text);
    println!("=== Extraction Prompt ===");
    println!("{}", &prompt[..prompt.len().min(500)]);

    // Simulated LLM response (as if the model responded to our prompt)
    let mock_response = r#"```json
{
  "entities": [
    {"name": "Alice Johnson", "type": "Person", "description": "CEO of TechCorp"},
    {"name": "TechCorp", "type": "Organization", "description": "A software company based in Seattle"},
    {"name": "Bob Smith", "type": "Person", "description": "CTO of TechCorp"},
    {"name": "DataInc", "type": "Organization", "description": "Company where Alice previously worked"},
    {"name": "CloudBase", "type": "Organization", "description": "Company acquired by TechCorp"}
  ],
  "relationships": [
    {"source": "Alice Johnson", "target": "TechCorp", "type": "CEO_OF", "description": "Alice is the CEO"},
    {"source": "Bob Smith", "target": "TechCorp", "type": "CTO_OF", "description": "Bob is the CTO"},
    {"source": "Alice Johnson", "target": "DataInc", "type": "WORKED_AT", "description": "Previously employed"},
    {"source": "TechCorp", "target": "CloudBase", "type": "ACQUIRED", "description": "Acquired for $50M"}
  ]
}
```"#;

    match parse_extraction_response(mock_response) {
        Ok(result) => {
            println!("\n=== Extracted Entities ===");
            for e in &result.entities {
                println!("  [{}] {} - {}", e.entity_type, e.name, e.description);
            }
            println!("\n=== Extracted Relationships ===");
            for r in &result.relationships {
                println!("  {} --[{}]--> {}", r.source, r.relation_type, r.target);
            }
        }
        Err(e) => println!("Parse error: {}", e),
    }
}
```
:::

::: hint level=1 title="Stripping code fences"
Look for lines that start with triple backticks and skip them:

```rust
fn strip_code_fences(text: &str) -> &str {
    let trimmed = text.trim();
    if trimmed.starts_with("```") {
        // Find end of first fence line
        let start = trimmed.find('\n').map(|i| i + 1).unwrap_or(0);
        // Find start of last fence line
        let end = trimmed.rfind("```").unwrap_or(trimmed.len());
        &trimmed[start..end]
    } else {
        trimmed
    }
}
```
:::

::: hint level=2 title="Simple JSON parsing approach"
Instead of building a full JSON parser, split the content into sections by
finding the `"entities"` and `"relationships"` arrays, then parse each
object by extracting key-value pairs line by line:

```rust
// Find array content between [ and ]
fn extract_array_content(json: &str, array_key: &str) -> Option<String> {
    let key_pattern = format!("\"{}\"", array_key);
    let key_pos = json.find(&key_pattern)?;
    let after_key = &json[key_pos..];
    let bracket_start = after_key.find('[')?;
    let bracket_content = &after_key[bracket_start..];

    // Find matching close bracket (handle nesting)
    let mut depth = 0;
    for (i, c) in bracket_content.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(bracket_content[1..i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}
```
:::

::: solution
```rust
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
struct Entity {
    name: String,
    entity_type: String,
    description: String,
}

#[derive(Debug, Clone, PartialEq)]
struct Relationship {
    source: String,
    target: String,
    relation_type: String,
    description: String,
}

#[derive(Debug, Clone)]
struct ExtractionResult {
    entities: Vec<Entity>,
    relationships: Vec<Relationship>,
}

fn build_extraction_prompt(text: &str) -> String {
    format!(
        r#"You are an entity and relationship extraction system. Analyze the following text and extract all named entities and their relationships.

Output your response as a JSON object with this exact schema:
{{
  "entities": [
    {{"name": "Entity Name", "type": "EntityType", "description": "Brief description"}}
  ],
  "relationships": [
    {{"source": "Source Entity", "target": "Target Entity", "type": "RELATION_TYPE", "description": "Brief description"}}
  ]
}}

Entity types to look for: Person, Organization, Location, Event, Date, Amount.
Relationship types should be in UPPER_SNAKE_CASE (e.g., WORKS_AT, CEO_OF, LOCATED_IN).

Example:
Input: "John is a manager at Google in Mountain View."
Output:
{{
  "entities": [
    {{"name": "John", "type": "Person", "description": "A manager at Google"}},
    {{"name": "Google", "type": "Organization", "description": "Technology company"}},
    {{"name": "Mountain View", "type": "Location", "description": "City where Google is located"}}
  ],
  "relationships": [
    {{"source": "John", "target": "Google", "type": "WORKS_AT", "description": "John is a manager at Google"}},
    {{"source": "Google", "target": "Mountain View", "type": "LOCATED_IN", "description": "Google is in Mountain View"}}
  ]
}}

Now extract entities and relationships from the following text:

{text}"#
    )
}

fn strip_code_fences(text: &str) -> &str {
    let trimmed = text.trim();
    if trimmed.starts_with("```") {
        let start = trimmed.find('\n').map(|i| i + 1).unwrap_or(0);
        let end = trimmed.rfind("```").unwrap_or(trimmed.len());
        trimmed[start..end].trim()
    } else {
        trimmed
    }
}

fn extract_json_string_value(line: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\"", key);
    let key_pos = line.find(&pattern)?;
    let after_key = &line[key_pos + pattern.len()..];

    // Skip colon and whitespace
    let after_colon = after_key.find(':').map(|i| &after_key[i + 1..])?;
    let trimmed = after_colon.trim();

    if !trimmed.starts_with('"') {
        return None;
    }

    let content = &trimmed[1..];
    // Find closing quote (handle escaped quotes)
    let mut end = 0;
    let mut prev_backslash = false;
    for (i, c) in content.char_indices() {
        if c == '"' && !prev_backslash {
            end = i;
            break;
        }
        prev_backslash = c == '\\';
    }

    Some(content[..end].to_string())
}

fn extract_array_content(json: &str, array_key: &str) -> Option<String> {
    let key_pattern = format!("\"{}\"", array_key);
    let key_pos = json.find(&key_pattern)?;
    let after_key = &json[key_pos..];
    let bracket_start = after_key.find('[')?;
    let bracket_content = &after_key[bracket_start..];

    let mut depth = 0;
    for (i, c) in bracket_content.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(bracket_content[1..i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_objects(array_content: &str) -> Vec<HashMap<String, String>> {
    let mut objects = Vec::new();
    let mut depth = 0;
    let mut obj_start = None;

    for (i, c) in array_content.char_indices() {
        match c {
            '{' => {
                if depth == 0 {
                    obj_start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = obj_start {
                        let obj_str = &array_content[start..=i];
                        let mut map = HashMap::new();
                        for line in obj_str.lines() {
                            for key in &["name", "type", "description", "source", "target"] {
                                if let Some(val) = extract_json_string_value(line, key) {
                                    map.insert(key.to_string(), val);
                                }
                            }
                        }
                        objects.push(map);
                    }
                }
            }
            _ => {}
        }
    }
    objects
}

fn deduplicate_entities(entities: Vec<Entity>) -> Vec<Entity> {
    let mut seen: HashMap<String, Entity> = HashMap::new();

    for entity in entities {
        let key = entity.name.trim().to_lowercase();
        if let Some(existing) = seen.get(&key) {
            if entity.description.len() > existing.description.len() {
                seen.insert(key, entity);
            }
        } else {
            seen.insert(key, entity);
        }
    }

    seen.into_values().collect()
}

fn parse_extraction_response(response: &str) -> Result<ExtractionResult, String> {
    let json = strip_code_fences(response);

    // Parse entities
    let entities_content = extract_array_content(json, "entities")
        .ok_or_else(|| "Could not find 'entities' array in response".to_string())?;
    let entity_objects = parse_objects(&entities_content);

    let mut entities: Vec<Entity> = entity_objects
        .into_iter()
        .filter_map(|obj| {
            let name = obj.get("name")?.clone();
            let entity_type = obj.get("type").cloned().unwrap_or_default();
            let description = obj.get("description").cloned().unwrap_or_default();
            Some(Entity {
                name,
                entity_type,
                description,
            })
        })
        .collect();

    entities = deduplicate_entities(entities);

    // Parse relationships
    let relationships_content = extract_array_content(json, "relationships")
        .ok_or_else(|| "Could not find 'relationships' array in response".to_string())?;
    let rel_objects = parse_objects(&relationships_content);

    let relationships: Vec<Relationship> = rel_objects
        .into_iter()
        .filter_map(|obj| {
            let source = obj.get("source")?.clone();
            let target = obj.get("target")?.clone();
            let relation_type = obj.get("type").cloned().unwrap_or_default();
            let description = obj.get("description").cloned().unwrap_or_default();
            Some(Relationship {
                source,
                target,
                relation_type,
                description,
            })
        })
        .collect();

    Ok(ExtractionResult {
        entities,
        relationships,
    })
}

fn main() {
    let text = r#"
Alice Johnson is the CEO of TechCorp, a software company based in Seattle.
She previously worked at DataInc where she met Bob Smith, who is now the
CTO of TechCorp. TechCorp recently acquired CloudBase for $50 million.
"#;

    let prompt = build_extraction_prompt(text);
    println!("=== Extraction Prompt ===");
    println!("{}", &prompt[..prompt.len().min(500)]);

    let mock_response = r#"```json
{
  "entities": [
    {"name": "Alice Johnson", "type": "Person", "description": "CEO of TechCorp"},
    {"name": "TechCorp", "type": "Organization", "description": "A software company based in Seattle"},
    {"name": "Bob Smith", "type": "Person", "description": "CTO of TechCorp"},
    {"name": "DataInc", "type": "Organization", "description": "Company where Alice previously worked"},
    {"name": "CloudBase", "type": "Organization", "description": "Company acquired by TechCorp"}
  ],
  "relationships": [
    {"source": "Alice Johnson", "target": "TechCorp", "type": "CEO_OF", "description": "Alice is the CEO"},
    {"source": "Bob Smith", "target": "TechCorp", "type": "CTO_OF", "description": "Bob is the CTO"},
    {"source": "Alice Johnson", "target": "DataInc", "type": "WORKED_AT", "description": "Previously employed"},
    {"source": "TechCorp", "target": "CloudBase", "type": "ACQUIRED", "description": "Acquired for $50M"}
  ]
}
```"#;

    match parse_extraction_response(mock_response) {
        Ok(result) => {
            println!("\n=== Extracted Entities ===");
            for e in &result.entities {
                println!("  [{}] {} - {}", e.entity_type, e.name, e.description);
            }
            println!("\n=== Extracted Relationships ===");
            for r in &result.relationships {
                println!("  {} --[{}]--> {}", r.source, r.relation_type, r.target);
            }
        }
        Err(e) => println!("Parse error: {}", e),
    }
}
```

### Explanation

The prompt template follows key principles: it specifies the exact JSON schema
expected, provides entity type and relationship type guidance, includes a
concrete few-shot example, and clearly separates the instruction from the input
text.

The parser is deliberately defensive. It first strips markdown code fences
(a very common LLM behavior), then locates the JSON arrays by matching
brackets with depth tracking. Individual objects are parsed by extracting
key-value pairs line by line. This approach is more robust than strict JSON
parsing because it tolerates minor formatting variations.

Entity deduplication uses a case-insensitive key (lowercase, trimmed name)
and keeps the entity with the longer description when duplicates are found,
preserving the richest metadata.
:::

::: tests mode=local
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_prompt_contains_text() {
        let prompt = build_extraction_prompt("Hello world");
        assert!(prompt.contains("Hello world"), "Prompt should include the input text");
    }

    #[test]
    fn test_build_prompt_contains_schema() {
        let prompt = build_extraction_prompt("test");
        assert!(prompt.contains("entities"), "Prompt should mention entities");
        assert!(prompt.contains("relationships"), "Prompt should mention relationships");
    }

    #[test]
    fn test_strip_code_fences_with_json() {
        let input = "```json\n{\"key\": \"value\"}\n```";
        let result = strip_code_fences(input);
        assert!(result.contains("\"key\""), "Should strip fences");
        assert!(!result.contains("```"), "Should not contain fence markers");
    }

    #[test]
    fn test_strip_code_fences_without_fences() {
        let input = "{\"key\": \"value\"}";
        let result = strip_code_fences(input);
        assert_eq!(result, input, "Should return input unchanged");
    }

    #[test]
    fn test_extract_json_string_value() {
        let line = r#"    "name": "Alice Johnson","#;
        let result = extract_json_string_value(line, "name");
        assert_eq!(result, Some("Alice Johnson".to_string()));
    }

    #[test]
    fn test_parse_basic_response() {
        let response = r#"{
  "entities": [
    {"name": "Alice", "type": "Person", "description": "A person"}
  ],
  "relationships": [
    {"source": "Alice", "target": "Bob", "type": "KNOWS", "description": "Friends"}
  ]
}"#;
        let result = parse_extraction_response(response).unwrap();
        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.entities[0].name, "Alice");
        assert_eq!(result.relationships.len(), 1);
        assert_eq!(result.relationships[0].relation_type, "KNOWS");
    }

    #[test]
    fn test_parse_response_with_code_fences() {
        let response = "```json\n{\"entities\": [{\"name\": \"X\", \"type\": \"T\", \"description\": \"D\"}], \"relationships\": []}\n```";
        let result = parse_extraction_response(response).unwrap();
        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.entities[0].name, "X");
    }

    #[test]
    fn test_deduplicate_entities_by_name() {
        let entities = vec![
            Entity { name: "Alice".into(), entity_type: "Person".into(), description: "Short".into() },
            Entity { name: "alice".into(), entity_type: "Person".into(), description: "A longer description".into() },
        ];
        let deduped = deduplicate_entities(entities);
        assert_eq!(deduped.len(), 1, "Should deduplicate by lowercase name");
        assert!(deduped[0].description.len() > 5, "Should keep the longer description");
    }

    #[test]
    fn test_parse_invalid_response() {
        let result = parse_extraction_response("this is not json at all");
        assert!(result.is_err(), "Should return error for invalid input");
    }
}
```
:::

::: reflection
- How would you improve extraction quality for domain-specific text (e.g., legal documents or scientific papers)?
- What are the tradeoffs between using structured output (JSON mode) vs free-form text extraction?
- How would you handle entity coreference (e.g., "Alice", "she", "the CEO" all referring to the same person)?
:::
