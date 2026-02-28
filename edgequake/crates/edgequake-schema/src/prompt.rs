//! Schema suggestion prompt templates and JSON schema definitions.
//!
//! Provides both the legacy single-pass prompt (`build_schema_prompt`) and the
//! new two-pass approach:
//! - **Pass 1** (`build_entity_discovery_prompt`): Discover entity types only
//! - **Pass 2** (`build_relationship_discovery_prompt`): Discover relationship types
//!   with full entity context from Pass 1
//!
//! Also provides persona auto-generation (`build_persona_prompt`) and few-shot
//! example generation from sampled documents.

use crate::analyzer::RawEntityType;
use crate::sampler::SampledDocument;
use crate::types::SuggestSchemaInput;

// ---------------------------------------------------------------------------
// Helper: truncate document content for prompt inclusion
// ---------------------------------------------------------------------------

/// Truncate a document's content at the given char limit.
fn truncate_content(content: &str, max_chars: usize) -> String {
    if content.len() > max_chars {
        format!("{}...[truncated]", &content[..max_chars])
    } else {
        content.to_string()
    }
}

/// Format sampled documents into a numbered text block for prompt inclusion.
fn format_documents(documents: &[SampledDocument], max_chars_per_doc: usize) -> String {
    let mut sample_text = String::new();
    for (i, doc) in documents.iter().enumerate() {
        sample_text.push_str(&format!(
            "### Document {} (source: {})\n\n{}\n\n---\n\n",
            i + 1,
            doc.source,
            truncate_content(&doc.content, max_chars_per_doc),
        ));
    }
    sample_text
}

// ---------------------------------------------------------------------------
// Legacy single-pass prompt (backward-compatible)
// ---------------------------------------------------------------------------

/// Build the schema analysis prompt from sampled documents and an optional domain hint.
///
/// The prompt instructs the LLM to analyze the documents and propose entity and
/// relation types with frequency estimates.
///
/// **Legacy function** — kept for backward compatibility. New code should use
/// `build_entity_discovery_prompt` + `build_relationship_discovery_prompt`.
pub fn build_schema_prompt(documents: &[SampledDocument], domain_hint: Option<&str>) -> String {
    let domain_hint_section = match domain_hint {
        Some(hint) => format!("This dataset is from the **{}** domain. Focus on entity and relation types that are specific and relevant to this domain.\n\n", hint),
        None => String::new(),
    };

    let sample_text = format_documents(documents, 8000);

    format!(
        r#"You are a Knowledge Graph Schema Analyst. Analyze the following sample documents from a dataset and propose entity types and relation types that would best capture the information in this domain.

{domain_hint_section}## Instructions

1. Read all sample documents carefully and identify recurring patterns of:
   - Named entities (people, organizations, places, concepts, domain-specific items)
   - Relationships between entities (actions, associations, hierarchies)

2. For each proposed entity type:
   - Use UPPER_SNAKE_CASE naming (e.g., PERSON, LEGAL_CASE, MEDICAL_PROCEDURE, PRODUCT)
   - Provide a clear description of what this type represents
   - Estimate how many of the {doc_count} sample documents contain this type

3. For each proposed relation type:
   - Use lower_snake_case naming (e.g., employs, filed_in, diagnosed_with)
   - Provide a description of the relationship. The relationship MUST be an action or structural association (a verb), NOT a specific value, metric, duration, or string literal (e.g., do NOT extract "five_days", "1000_designs", or "revenue" as relationship types).
   - Specify which entity types can be the source and target
   - Estimate how many sample documents exhibit this relation

4. ALWAYS include these baseline entity types: PERSON, ORGANIZATION, LOCATION, DATE
   You may add domain-specific entity types beyond these, such as products or services.

5. Aim for 8-25 entity types and 10-30 relation types. Consider domain-specific types such as PRODUCT, SERVICE, EVENT, DOCUMENT, FINANCIAL_INSTRUMENT, TECHNOLOGY beyond the baselines. Be specific rather than generic.

6. Return your response as valid JSON matching the required schema. Do not include any text outside the JSON.

## Required JSON Schema

Your response must be a JSON object with this structure:
```json
{{
  "entity_types": [
    {{
      "name": "UPPER_SNAKE_CASE type name",
      "description": "What this entity type represents",
      "frequency": 0
    }}
  ],
  "relation_types": [
    {{
      "name": "lower_snake_case relation name",
      "description": "What this relation represents",
      "source_type": "Source entity type",
      "target_type": "Target entity type",
      "frequency": 0
    }}
  ]
}}
```

## Sample Documents ({doc_count} documents)

{sample_text}"#,
        domain_hint_section = domain_hint_section,
        doc_count = documents.len(),
        sample_text = sample_text,
    )
}

/// Build the JSON schema for OpenAI Structured Outputs (legacy single-pass).
///
/// Defines the expected response format with entity types and relation types.
pub fn build_json_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "schema_proposal",
        "strict": true,
        "schema": {
            "type": "object",
            "properties": {
                "entity_types": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "UPPER_SNAKE_CASE type name"
                            },
                            "description": {
                                "type": "string",
                                "description": "What this entity type represents"
                            },
                            "frequency": {
                                "type": "integer",
                                "description": "Estimated documents containing this type"
                            }
                        },
                        "required": ["name", "description", "frequency"],
                        "additionalProperties": false
                    }
                },
                "relation_types": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "lower_snake_case relation name"
                            },
                            "description": {
                                "type": "string",
                                "description": "What this relation represents"
                            },
                            "source_type": {
                                "type": "string",
                                "description": "Source entity type"
                            },
                            "target_type": {
                                "type": "string",
                                "description": "Target entity type"
                            },
                            "frequency": {
                                "type": "integer",
                                "description": "Estimated documents exhibiting this relation"
                            }
                        },
                        "required": ["name", "description", "source_type", "target_type", "frequency"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["entity_types", "relation_types"],
            "additionalProperties": false
        }
    })
}

// ===========================================================================
// Two-pass prompt system (new)
// ===========================================================================

// ---------------------------------------------------------------------------
// A: Persona generation
// ---------------------------------------------------------------------------

/// Build a prompt that asks the LLM to identify the domain from document excerpts
/// and generate a specialist persona.
///
/// This is a short, cheap call (small input from excerpts, small JSON output).
pub fn build_persona_prompt(documents: &[SampledDocument]) -> String {
    let mut excerpts = String::new();
    for (i, doc) in documents.iter().take(10).enumerate() {
        let excerpt = if doc.content.len() > 500 {
            &doc.content[..500]
        } else {
            &doc.content
        };
        excerpts.push_str(&format!(
            "Excerpt {} (source: {}):\n{}\n\n",
            i + 1,
            doc.source,
            excerpt,
        ));
    }

    format!(
        r#"Read these document excerpts and identify the domain they belong to.

Respond with a JSON object:
{{
  "domain": "brief domain name",
  "persona": "You are a [domain] specialist who analyzes documents to identify key entities and relationships for a knowledge graph. [2-3 sentences about what makes this domain unique and what to look for].",
  "key_themes": ["theme1", "theme2", "theme3"]
}}

Base your response ONLY on the excerpts provided. Do not guess about content not shown.

## Document Excerpts

{excerpts}"#,
        excerpts = excerpts,
    )
}

/// Build the JSON schema for the persona generation response.
pub fn build_persona_json_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "persona_result",
        "strict": true,
        "schema": {
            "type": "object",
            "properties": {
                "domain": {
                    "type": "string",
                    "description": "Brief domain name (e.g. 'legal', 'healthcare', 'finance')"
                },
                "persona": {
                    "type": "string",
                    "description": "A specialist persona description for the domain"
                },
                "key_themes": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Key themes found in the documents"
                }
            },
            "required": ["domain", "persona", "key_themes"],
            "additionalProperties": false
        }
    })
}

// ---------------------------------------------------------------------------
// B: Few-shot example generation
// ---------------------------------------------------------------------------

/// Return curated few-shot examples that demonstrate domain-specific entity
/// and relationship discovery.
///
/// Unlike the old `generate_few_shot_examples` (which produced the same
/// generic labels for every snippet), these hardcoded worked examples teach
/// the LLM to produce domain-specific entity types and verb-phrase
/// relationship types.
fn curated_few_shot_section() -> &'static str {
    r#"## Few-Shot Examples

Here are worked examples showing the level of specificity and naming conventions expected.

**Example 1 — Legal / Financial domain:**
Given text: "Ghislaine Maxwell transferred $2.4M from JP Morgan account #4829 to a shell company registered in the British Virgin Islands..."

Entity types discovered:
- PERSON (named individuals)
- ORGANIZATION (companies, institutions)
- FINANCIAL_ACCOUNT (bank accounts, investment accounts)
- SHELL_COMPANY (entities created for asset concealment)
- JURISDICTION (legal territories, regulatory zones)
- MONETARY_AMOUNT (sums of money)

Relationship types discovered:
- transferred_funds (PERSON → FINANCIAL_ACCOUNT): moved money between accounts
- holds_account_at (PERSON → ORGANIZATION): maintains a financial account
- registered_in (SHELL_COMPANY → JURISDICTION): legal registration location
- controlled_by (SHELL_COMPANY → PERSON): beneficial ownership

Notice: relationships are VERB PHRASES describing how entities connect, NOT nouns like `related_to` or `money` or `account`.

**Example 2 — Technology / Business domain:**
Given text: "Samsung's Austin semiconductor fab produces 3nm chips supplied to Apple for the iPhone 16 Pro, competing with TSMC in the mobile processor market..."

Entity types discovered:
- ORGANIZATION (companies)
- PRODUCT (manufactured goods, product lines)
- FACILITY (factories, offices, data centers)
- TECHNOLOGY (technical standards, processes)
- MARKET_SEGMENT (industry verticals, market categories)

Relationship types discovered:
- manufactures (ORGANIZATION → PRODUCT): produces or fabricates
- supplied_to (PRODUCT → ORGANIZATION): sold or delivered to
- deployed_at (TECHNOLOGY → FACILITY): technology used at location
- competes_in (ORGANIZATION → MARKET_SEGMENT): market competition
- operates (ORGANIZATION → FACILITY): owns or runs a facility

Notice: `market_share` would be WRONG (it's a metric, not a relationship). `product` would be WRONG (it's an entity type, not a relationship). Always ask: HOW are these two entities connected?

Apply similar domain-specific specificity to the actual documents below.

"#
}

// ---------------------------------------------------------------------------
// C: Entity discovery prompt (Pass 1)
// ---------------------------------------------------------------------------

/// Build the Pass 1 prompt that discovers entity types only.
///
/// Includes the auto-generated (or user-provided) persona, few-shot examples,
/// user context, and sampled documents. Does NOT ask for relationship types.
pub fn build_entity_discovery_prompt(
    documents: &[SampledDocument],
    persona: &str,
    input: &SuggestSchemaInput,
) -> String {
    let mut prompt = String::new();

    // Persona preamble
    prompt.push_str(&format!("{}\n\n", persona));

    // User context section
    if let Some(ref desc) = input.domain_description {
        prompt.push_str(&format!(
            "## User Context\n\nThe user describes this dataset as: **{}**\n\n",
            desc
        ));
    } else if let Some(ref hint) = input.domain_hint {
        prompt.push_str(&format!(
            "## User Context\n\nThis dataset is from the **{}** domain.\n\n",
            hint
        ));
    }

    // Curated few-shot examples
    prompt.push_str(curated_few_shot_section());

    // User-expected entity types
    if let Some(ref expected) = input.expected_entity_types {
        if !expected.is_empty() {
            prompt.push_str(&format!(
                "## Required Entity Types\n\n\
                 The user expects these entity types to be present: [{}]. \
                 ALWAYS include these in your output, plus any additional types you discover.\n\n",
                expected.join(", ")
            ));
        }
    }

    // Instructions
    prompt.push_str(&format!(
        r#"## Task: Discover Entity Types

Analyze the following {doc_count} sample documents and identify all meaningful entity types for a knowledge graph.

### Instructions

1. Read all documents carefully. Look for recurring named entities: people, organizations, places, dates, concepts, products, services, and domain-specific items.

2. For each entity type:
   - Use UPPER_SNAKE_CASE naming (e.g., PERSON, LEGAL_CASE, FINANCIAL_ACCOUNT, PRODUCT)
   - Provide a clear description of what this type represents
   - Estimate how many of the {doc_count} documents contain this type

3. ALWAYS include baseline types: PERSON, ORGANIZATION, LOCATION, DATE

4. Aim for 8-25 entity types. Consider domain-specific types such as PRODUCT, SERVICE, EVENT, DOCUMENT, FINANCIAL_INSTRUMENT, TECHNOLOGY beyond the baselines. Be specific to the dataset rather than generic.

5. Return ONLY entity types. Do NOT include relationship types in this response.

"#,
        doc_count = documents.len(),
    ));

    // Document text
    prompt.push_str(&format!(
        "## Sample Documents ({} documents)\n\n{}",
        documents.len(),
        format_documents(documents, 8000),
    ));

    prompt
}

/// Build the JSON schema for the entity-only discovery response (Pass 1).
pub fn build_entity_json_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "entity_discovery",
        "strict": true,
        "schema": {
            "type": "object",
            "properties": {
                "entity_types": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "UPPER_SNAKE_CASE entity type name"
                            },
                            "description": {
                                "type": "string",
                                "description": "What this entity type represents"
                            },
                            "frequency": {
                                "type": "integer",
                                "description": "Estimated documents containing this type"
                            }
                        },
                        "required": ["name", "description", "frequency"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["entity_types"],
            "additionalProperties": false
        }
    })
}

// ---------------------------------------------------------------------------
// D: Relationship discovery prompt (Pass 2)
// ---------------------------------------------------------------------------

/// Build the Pass 2 prompt that discovers relationship types given known entity types.
///
/// Includes the persona, discovered entity types as context, user context,
/// and sampled documents. Source and target types must come from the entity list.
pub fn build_relationship_discovery_prompt(
    documents: &[SampledDocument],
    persona: &str,
    input: &SuggestSchemaInput,
    discovered_entities: &[RawEntityType],
) -> String {
    let mut prompt = String::new();

    // Persona preamble
    prompt.push_str(&format!("{}\n\n", persona));

    // Discovered entity types as context
    prompt.push_str("## Known Entity Types\n\n");
    prompt.push_str(
        "The following entity types have been identified for this dataset's knowledge graph:\n\n",
    );
    for et in discovered_entities {
        prompt.push_str(&format!("- **{}**: {}\n", et.name, et.description));
    }
    prompt.push('\n');

    // User context section
    if let Some(ref desc) = input.domain_description {
        prompt.push_str(&format!(
            "## User Context\n\nThe user describes this dataset as: **{}**\n\n",
            desc
        ));
    } else if let Some(ref hint) = input.domain_hint {
        prompt.push_str(&format!(
            "## User Context\n\nThis dataset is from the **{}** domain.\n\n",
            hint
        ));
    }

    // User-expected relationship types
    if let Some(ref expected) = input.expected_relationship_types {
        if !expected.is_empty() {
            prompt.push_str(&format!(
                "## Required Relationship Types\n\n\
                 The user expects these relationship types to be present: [{}]. \
                 ALWAYS include these in your output, plus any additional types you discover.\n\n",
                expected.join(", ")
            ));
        }
    }

    // Instructions
    prompt.push_str(&format!(
        r#"## Task: Discover Relationship Types

Analyze the following {doc_count} sample documents and identify all meaningful relationship types between the known entity types above.

### Instructions

1. Read all documents carefully. Look for actions, associations, hierarchies, and connections between entities.

2. For each relationship type:
   - Use lower_snake_case naming (e.g., employs, filed_in, diagnosed_with, acquired)
   - The relationship MUST be an action or structural association (a verb or prepositional phrase), NOT a specific value, metric, duration, or string literal. Do NOT extract specific statements like "five_days", "1000_designs", or "revenue" as relationship types.
   - Provide a description of the relationship
   - Specify source_type and target_type from the entity types listed above
   - Estimate how many of the {doc_count} documents exhibit this relationship

3. **Blocklist — NEVER use these catch-all types:** `related_to`, `associated_with`, `has_connection`, `connected_to`, `linked_to`, `involves`, `pertains_to`, `concerns`. These are meaningless. Instead ask: HOW are the two entities related? Name the specific action or structural link.

4. **Anti-pattern examples:**
   - WRONG: `market_share`, `product`, `idea`, `Video game industry`, `half_sister`
   - RIGHT: `holds_market_share_in`, `manufactures`, `conceived`, `operates_in`, `sibling_of`
   - If the name reads as a noun, it is probably a missing entity type, not a relationship.

5. **Moderate generality:** Prefer `family_member_of` or `sibling_of` over hyper-specific types like `half_sister`, `brother`, `stepfather`. Group closely related sub-types into one relationship type.

6. Source and target types MUST be from the entity types listed above.

7. Aim for 10-30 relationship types. Be specific to the dataset rather than generic, but ensure they represent generic relationship classes, not specific instance data.

8. Return ONLY relationship types. Do NOT include entity types in this response.

"#,
        doc_count = documents.len(),
    ));

    // Document text
    prompt.push_str(&format!(
        "## Sample Documents ({} documents)\n\n{}",
        documents.len(),
        format_documents(documents, 8000),
    ));

    prompt
}

/// Build the JSON schema for the relationship-only discovery response (Pass 2).
pub fn build_relationship_json_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "relationship_discovery",
        "strict": true,
        "schema": {
            "type": "object",
            "properties": {
                "relation_types": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "lower_snake_case relationship name"
                            },
                            "description": {
                                "type": "string",
                                "description": "What this relationship represents"
                            },
                            "source_type": {
                                "type": "string",
                                "description": "Source entity type (from known types)"
                            },
                            "target_type": {
                                "type": "string",
                                "description": "Target entity type (from known types)"
                            },
                            "frequency": {
                                "type": "integer",
                                "description": "Estimated documents exhibiting this relationship"
                            }
                        },
                        "required": ["name", "description", "source_type", "target_type", "frequency"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["relation_types"],
            "additionalProperties": false
        }
    })
}

// ---------------------------------------------------------------------------
// E: Consolidation / review prompt (Pass 3)
// ---------------------------------------------------------------------------

/// Build the Pass 3 consolidation prompt that reviews and cleans up the
/// combined entity + relationship lists.
///
/// Receives the discovered entity types, relationship types, and a handful
/// of short document excerpts. Asks the LLM to merge overly-specific types,
/// remove non-relationships (nouns, metrics), remove catch-all types, and
/// suggest missing entity types evident from the excerpts.
pub fn build_consolidation_prompt(
    entity_types: &[RawEntityType],
    relation_types: &[crate::analyzer::RawRelationType],
    documents: &[SampledDocument],
    persona: &str,
) -> String {
    let mut prompt = String::new();

    // Persona
    prompt.push_str(&format!("{}\n\n", persona));

    // Current entity types
    prompt.push_str("## Current Entity Types\n\n");
    for et in entity_types {
        prompt.push_str(&format!("- **{}**: {} (freq: {})\n", et.name, et.description, et.frequency));
    }
    prompt.push('\n');

    // Current relationship types
    prompt.push_str("## Current Relationship Types\n\n");
    for rt in relation_types {
        prompt.push_str(&format!(
            "- **{}**: {} [{} → {}] (freq: {})\n",
            rt.name, rt.description, rt.source_type, rt.target_type, rt.frequency
        ));
    }
    prompt.push('\n');

    // Short document excerpts for context
    prompt.push_str("## Document Excerpts (for reference)\n\n");
    for (i, doc) in documents.iter().take(5).enumerate() {
        let excerpt = if doc.content.len() > 500 {
            &doc.content[..500]
        } else {
            &doc.content
        };
        prompt.push_str(&format!("**Excerpt {}:** {}\n\n", i + 1, excerpt));
    }

    // Instructions
    prompt.push_str(r#"## Task: Consolidate and Clean Schema

Review the entity types and relationship types above. Produce a cleaned, consolidated version by applying these rules:

1. **Merge overly-specific types**: Combine closely related types into one (e.g., `half_sister` + `brother` + `stepfather` → `family_member_of` or `sibling_of` + `parent_of`).

2. **Remove non-relationships**: Delete any relationship type that is actually a noun, metric, or value rather than a verb phrase (e.g., `market_share`, `product`, `idea`, `Video game industry`).

3. **Remove catch-all types**: Delete meaningless catch-all relationship types: `related_to`, `associated_with`, `has_connection`, `connected_to`, `linked_to`, `involves`, `pertains_to`, `concerns`.

4. **Validate source/target**: Every relationship must have a valid source_type and target_type from the entity type list. Remove relationships with invalid entity references.

5. **Suggest missing entity types**: If the document excerpts clearly reference a category of entities not covered (e.g., PRODUCT, SERVICE, EVENT), add it.

6. **Preserve frequency estimates**: Keep the original frequency values where possible; for merged types, sum the frequencies.

Return the cleaned lists as JSON. Do NOT add relationships that weren't in the original list — only keep, merge, or remove.
"#);

    prompt
}

/// Build the JSON schema for the consolidation response (Pass 3).
///
/// Same structure as the legacy single-pass schema: entity_types + relation_types.
pub fn build_consolidation_json_schema() -> serde_json::Value {
    serde_json::json!({
        "name": "consolidation_result",
        "strict": true,
        "schema": {
            "type": "object",
            "properties": {
                "entity_types": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "UPPER_SNAKE_CASE entity type name"
                            },
                            "description": {
                                "type": "string",
                                "description": "What this entity type represents"
                            },
                            "frequency": {
                                "type": "integer",
                                "description": "Estimated documents containing this type"
                            }
                        },
                        "required": ["name", "description", "frequency"],
                        "additionalProperties": false
                    }
                },
                "relation_types": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "lower_snake_case relationship name"
                            },
                            "description": {
                                "type": "string",
                                "description": "What this relationship represents"
                            },
                            "source_type": {
                                "type": "string",
                                "description": "Source entity type"
                            },
                            "target_type": {
                                "type": "string",
                                "description": "Target entity type"
                            },
                            "frequency": {
                                "type": "integer",
                                "description": "Estimated documents exhibiting this relationship"
                            }
                        },
                        "required": ["name", "description", "source_type", "target_type", "frequency"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["entity_types", "relation_types"],
            "additionalProperties": false
        }
    })
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_docs(contents: &[&str]) -> Vec<SampledDocument> {
        contents
            .iter()
            .enumerate()
            .map(|(i, c)| SampledDocument {
                id: format!("doc{}", i + 1),
                content: c.to_string(),
                source: format!("test{}.parquet", i + 1),
            })
            .collect()
    }

    fn default_input() -> SuggestSchemaInput {
        SuggestSchemaInput::default()
    }

    // -----------------------------------------------------------------------
    // Legacy prompt tests (backward compatibility)
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_schema_prompt_without_hint() {
        let docs = make_docs(&["Test content"]);
        let prompt = build_schema_prompt(&docs, None);
        assert!(prompt.contains("Knowledge Graph Schema Analyst"));
        assert!(prompt.contains("Test content"));
        assert!(prompt.contains("1 documents"));
    }

    #[test]
    fn test_build_schema_prompt_with_hint() {
        let docs = make_docs(&["Legal filing text"]);
        let prompt = build_schema_prompt(&docs, Some("legal"));
        assert!(prompt.contains("legal"));
        assert!(prompt.contains("Legal filing text"));
    }

    #[test]
    fn test_build_json_schema_structure() {
        let schema = build_json_schema();
        assert!(schema.get("name").is_some());
        assert!(schema.get("schema").is_some());

        let inner = &schema["schema"];
        assert_eq!(inner["type"], "object");
        assert!(inner["properties"]["entity_types"].is_object());
        assert!(inner["properties"]["relation_types"].is_object());
    }

    #[test]
    fn test_long_document_truncation() {
        let long_content = "x".repeat(10000);
        let docs = vec![SampledDocument {
            id: "doc1".to_string(),
            content: long_content,
            source: "test.parquet".to_string(),
        }];
        let prompt = build_schema_prompt(&docs, None);
        assert!(prompt.contains("...[truncated]"));
    }

    // -----------------------------------------------------------------------
    // Persona prompt tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_persona_prompt_includes_excerpts() {
        let docs = make_docs(&[
            "John Smith filed a lawsuit against Acme Corp in the Southern District of New York.",
            "The defendant corporation was incorporated in Delaware in 2015.",
        ]);
        let prompt = build_persona_prompt(&docs);
        assert!(prompt.contains("John Smith filed a lawsuit"));
        assert!(prompt.contains("defendant corporation"));
        assert!(prompt.contains("Document Excerpts"));
        assert!(prompt.contains("Excerpt 1"));
        assert!(prompt.contains("Excerpt 2"));
    }

    #[test]
    fn test_build_persona_prompt_truncates_long_excerpts() {
        let long = "a".repeat(1000);
        let docs = make_docs(&[&long]);
        let prompt = build_persona_prompt(&docs);
        // Should only include first 500 chars
        assert!(prompt.contains(&"a".repeat(500)));
        assert!(!prompt.contains(&"a".repeat(600)));
    }

    #[test]
    fn test_build_persona_prompt_limits_to_10_docs() {
        let contents: Vec<String> = (0..15).map(|i| format!("Document content {}", i)).collect();
        let content_refs: Vec<&str> = contents.iter().map(|s| s.as_str()).collect();
        let docs = make_docs(&content_refs);
        let prompt = build_persona_prompt(&docs);
        assert!(prompt.contains("Excerpt 10"));
        assert!(!prompt.contains("Excerpt 11"));
    }

    // -----------------------------------------------------------------------
    // Entity discovery prompt tests (Pass 1)
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_entity_discovery_prompt_with_persona() {
        let docs = make_docs(&["Some legal document text about contracts."]);
        let persona = "You are a legal domain specialist who analyzes court documents.";
        let input = default_input();

        let prompt = build_entity_discovery_prompt(&docs, persona, &input);

        // Starts with persona
        assert!(prompt.starts_with(persona));
        // Contains entity-specific instructions
        assert!(prompt.contains("Discover Entity Types"));
        assert!(prompt.contains("ALWAYS include baseline types"));
        // Does NOT ask for relationship types
        assert!(prompt.contains("Do NOT include relationship types"));
        // Contains document text
        assert!(prompt.contains("Some legal document text"));
    }

    #[test]
    fn test_build_entity_discovery_prompt_with_user_entities() {
        let docs = make_docs(&["Text about vehicles and drivers."]);
        let persona = "You are a specialist.";
        let input = SuggestSchemaInput {
            expected_entity_types: Some(vec!["VEHICLE".to_string(), "DRIVER".to_string()]),
            ..Default::default()
        };

        let prompt = build_entity_discovery_prompt(&docs, persona, &input);

        assert!(prompt.contains("VEHICLE, DRIVER"));
        assert!(prompt.contains("ALWAYS include these in your output"));
    }

    #[test]
    fn test_build_entity_discovery_prompt_with_domain_description() {
        let docs = make_docs(&["Insurance claim data."]);
        let persona = "You are a specialist.";
        let input = SuggestSchemaInput {
            domain_description: Some("insurance claims processing".to_string()),
            ..Default::default()
        };

        let prompt = build_entity_discovery_prompt(&docs, persona, &input);
        assert!(prompt.contains("insurance claims processing"));
    }

    #[test]
    fn test_build_entity_discovery_prompt_with_domain_hint() {
        let docs = make_docs(&["Finance data."]);
        let persona = "You are a specialist.";
        let input = SuggestSchemaInput {
            domain_hint: Some("finance".to_string()),
            ..Default::default()
        };

        let prompt = build_entity_discovery_prompt(&docs, persona, &input);
        assert!(prompt.contains("finance"));
    }

    // -----------------------------------------------------------------------
    // Relationship discovery prompt tests (Pass 2)
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_relationship_discovery_prompt_includes_entity_context() {
        let docs = make_docs(&["Company ABC employs John and is based in NYC."]);
        let persona = "You are a domain specialist.";
        let input = default_input();
        let entities = vec![
            RawEntityType {
                name: "PERSON".to_string(),
                description: "Named individuals".to_string(),
                frequency: 5,
            },
            RawEntityType {
                name: "ORGANIZATION".to_string(),
                description: "Companies and institutions".to_string(),
                frequency: 3,
            },
        ];

        let prompt =
            build_relationship_discovery_prompt(&docs, persona, &input, &entities);

        assert!(prompt.contains("Known Entity Types"));
        assert!(prompt.contains("**PERSON**: Named individuals"));
        assert!(prompt.contains("**ORGANIZATION**: Companies and institutions"));
        assert!(prompt.contains("MUST be from the entity types listed above"));
        assert!(prompt.contains("Do NOT include entity types"));
    }

    #[test]
    fn test_build_relationship_discovery_prompt_with_user_rels() {
        let docs = make_docs(&["Some text."]);
        let persona = "You are a specialist.";
        let input = SuggestSchemaInput {
            expected_relationship_types: Some(vec![
                "manages".to_string(),
                "funds".to_string(),
            ]),
            ..Default::default()
        };
        let entities = vec![RawEntityType {
            name: "PERSON".to_string(),
            description: "People".to_string(),
            frequency: 1,
        }];

        let prompt =
            build_relationship_discovery_prompt(&docs, persona, &input, &entities);

        assert!(prompt.contains("manages, funds"));
        assert!(prompt.contains("ALWAYS include these in your output"));
    }

    // -----------------------------------------------------------------------
    // Curated few-shot example tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_curated_few_shot_contains_legal_example() {
        let section = curated_few_shot_section();
        assert!(section.contains("Legal / Financial domain"));
        assert!(section.contains("FINANCIAL_ACCOUNT"));
        assert!(section.contains("SHELL_COMPANY"));
        assert!(section.contains("transferred_funds"));
        assert!(section.contains("holds_account_at"));
    }

    #[test]
    fn test_curated_few_shot_contains_tech_example() {
        let section = curated_few_shot_section();
        assert!(section.contains("Technology / Business domain"));
        assert!(section.contains("PRODUCT"));
        assert!(section.contains("FACILITY"));
        assert!(section.contains("manufactures"));
        assert!(section.contains("competes_in"));
    }

    #[test]
    fn test_curated_few_shot_contains_anti_pattern_callouts() {
        let section = curated_few_shot_section();
        assert!(section.contains("related_to"));
        assert!(section.contains("VERB PHRASES"));
        assert!(section.contains("market_share"));
        assert!(section.contains("WRONG"));
    }

    // -----------------------------------------------------------------------
    // JSON schema structure tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_entity_json_schema_structure() {
        let schema = build_entity_json_schema();
        assert_eq!(schema["name"], "entity_discovery");
        assert_eq!(schema["strict"], true);

        let inner = &schema["schema"];
        assert_eq!(inner["type"], "object");
        assert!(inner["properties"]["entity_types"].is_object());

        let items = &inner["properties"]["entity_types"]["items"];
        let required = items["required"].as_array().unwrap();
        let names: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(names.contains(&"name"));
        assert!(names.contains(&"description"));
        assert!(names.contains(&"frequency"));

        // Should NOT have relation_types
        assert!(inner["properties"].get("relation_types").is_none());
    }

    #[test]
    fn test_relationship_json_schema_structure() {
        let schema = build_relationship_json_schema();
        assert_eq!(schema["name"], "relationship_discovery");
        assert_eq!(schema["strict"], true);

        let inner = &schema["schema"];
        assert_eq!(inner["type"], "object");
        assert!(inner["properties"]["relation_types"].is_object());

        let items = &inner["properties"]["relation_types"]["items"];
        let required = items["required"].as_array().unwrap();
        let names: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(names.contains(&"name"));
        assert!(names.contains(&"description"));
        assert!(names.contains(&"source_type"));
        assert!(names.contains(&"target_type"));
        assert!(names.contains(&"frequency"));

        // Should NOT have entity_types
        assert!(inner["properties"].get("entity_types").is_none());
    }

    #[test]
    fn test_persona_json_schema_structure() {
        let schema = build_persona_json_schema();
        assert_eq!(schema["name"], "persona_result");
        assert_eq!(schema["strict"], true);

        let inner = &schema["schema"];
        assert_eq!(inner["type"], "object");

        let required = inner["required"].as_array().unwrap();
        let names: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(names.contains(&"domain"));
        assert!(names.contains(&"persona"));
        assert!(names.contains(&"key_themes"));
    }

    // -----------------------------------------------------------------------
    // Consolidation prompt tests (Pass 3)
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_consolidation_prompt_includes_types() {
        let docs = make_docs(&["Some document text about companies."]);
        let entities = vec![
            RawEntityType {
                name: "PERSON".to_string(),
                description: "People".to_string(),
                frequency: 10,
            },
            RawEntityType {
                name: "ORGANIZATION".to_string(),
                description: "Companies".to_string(),
                frequency: 8,
            },
        ];
        let relations = vec![crate::analyzer::RawRelationType {
            name: "employs".to_string(),
            description: "Employment".to_string(),
            source_type: "ORGANIZATION".to_string(),
            target_type: "PERSON".to_string(),
            frequency: 5,
        }];
        let persona = "You are a domain specialist.";

        let prompt = build_consolidation_prompt(&entities, &relations, &docs, persona);

        assert!(prompt.contains("PERSON"));
        assert!(prompt.contains("ORGANIZATION"));
        assert!(prompt.contains("employs"));
        assert!(prompt.contains("Consolidate and Clean Schema"));
        assert!(prompt.contains("Merge overly-specific types"));
        assert!(prompt.contains("Remove catch-all types"));
        assert!(prompt.contains("related_to"));
    }

    #[test]
    fn test_consolidation_json_schema_structure() {
        let schema = build_consolidation_json_schema();
        assert_eq!(schema["name"], "consolidation_result");
        assert_eq!(schema["strict"], true);

        let inner = &schema["schema"];
        assert_eq!(inner["type"], "object");
        assert!(inner["properties"]["entity_types"].is_object());
        assert!(inner["properties"]["relation_types"].is_object());

        let required = inner["required"].as_array().unwrap();
        let names: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(names.contains(&"entity_types"));
        assert!(names.contains(&"relation_types"));
    }

    #[test]
    fn test_build_relationship_prompt_contains_blocklist() {
        let docs = make_docs(&["Some text."]);
        let persona = "You are a specialist.";
        let input = default_input();
        let entities = vec![RawEntityType {
            name: "PERSON".to_string(),
            description: "People".to_string(),
            frequency: 1,
        }];

        let prompt =
            build_relationship_discovery_prompt(&docs, persona, &input, &entities);

        assert!(prompt.contains("related_to"));
        assert!(prompt.contains("associated_with"));
        assert!(prompt.contains("Blocklist"));
        assert!(prompt.contains("Anti-pattern"));
        assert!(prompt.contains("Moderate generality"));
    }
}
