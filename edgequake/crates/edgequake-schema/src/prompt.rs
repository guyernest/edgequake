//! Schema suggestion prompt template and JSON schema definition.
//!
//! Builds the prompt sent to the LLM for schema analysis and defines the
//! structured output JSON schema for reliable parsing.

use crate::sampler::SampledDocument;

/// Build the schema analysis prompt from sampled documents and an optional domain hint.
///
/// The prompt instructs the LLM to analyze the documents and propose entity and
/// relation types with frequency estimates.
pub fn build_schema_prompt(documents: &[SampledDocument], domain_hint: Option<&str>) -> String {
    let domain_hint_section = match domain_hint {
        Some(hint) => format!("This dataset is from the **{}** domain. Focus on entity and relation types that are specific and relevant to this domain.\n\n", hint),
        None => String::new(),
    };

    let mut sample_text = String::new();
    for (i, doc) in documents.iter().enumerate() {
        sample_text.push_str(&format!(
            "### Document {} (source: {})\n\n{}\n\n---\n\n",
            i + 1,
            doc.source,
            // Truncate very long documents to avoid wasting context on a single doc
            if doc.content.len() > 8000 {
                format!("{}...[truncated]", &doc.content[..8000])
            } else {
                doc.content.clone()
            }
        ));
    }

    format!(
        r#"You are a Knowledge Graph Schema Analyst. Analyze the following sample documents from a dataset and propose entity types and relation types that would best capture the information in this domain.

{domain_hint_section}## Instructions

1. Read all sample documents carefully and identify recurring patterns of:
   - Named entities (people, organizations, places, concepts, domain-specific items)
   - Relationships between entities (actions, associations, hierarchies)

2. For each proposed entity type:
   - Use UPPER_SNAKE_CASE naming (e.g., PERSON, LEGAL_CASE, MEDICAL_PROCEDURE)
   - Provide a clear description of what this type represents
   - Estimate how many of the {doc_count} sample documents contain this type

3. For each proposed relation type:
   - Use lower_snake_case naming (e.g., employs, filed_in, diagnosed_with)
   - Provide a description of the relationship
   - Specify which entity types can be the source and target
   - Estimate how many sample documents exhibit this relation

4. ALWAYS include these baseline entity types: PERSON, ORGANIZATION, LOCATION, DATE
   You may add domain-specific entity types beyond these.

5. Aim for 5-15 entity types and 10-30 relation types. Be specific to the domain rather than generic.

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

/// Build the JSON schema for OpenAI Structured Outputs.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_schema_prompt_without_hint() {
        let docs = vec![SampledDocument {
            id: "doc1".to_string(),
            content: "Test content".to_string(),
            source: "test.parquet".to_string(),
        }];
        let prompt = build_schema_prompt(&docs, None);
        assert!(prompt.contains("Knowledge Graph Schema Analyst"));
        assert!(prompt.contains("Test content"));
        assert!(prompt.contains("1 documents"));
        assert!(!prompt.contains("domain"));
    }

    #[test]
    fn test_build_schema_prompt_with_hint() {
        let docs = vec![SampledDocument {
            id: "doc1".to_string(),
            content: "Legal filing text".to_string(),
            source: "legal.parquet".to_string(),
        }];
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
}
