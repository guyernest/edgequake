//! Post-processing normalization for raw LLM schema proposals.
//!
//! Normalizes type names, deduplicates entries, and ensures baseline
//! entity types are always present.

use std::collections::HashMap;

use crate::analyzer::RawSchemaProposal;
use crate::types::{
    EntityTypeProposal, RelationTypeProposal, SamplingMetadata, SchemaProposal, SchemaStatus,
    SuggestSchemaInput,
};

/// Baseline entity types that are always included in every proposal.
const BASELINE_ENTITY_TYPES: &[(&str, &str)] = &[
    ("PERSON", "Named individuals"),
    ("ORGANIZATION", "Companies, institutions, government agencies"),
    ("LOCATION", "Places, addresses, geographic regions"),
    ("DATE", "Dates, time periods, timestamps"),
];

/// Normalize a raw LLM schema proposal into a finalized SchemaProposal.
///
/// This function:
/// 1. Normalizes entity type names to UPPER_SNAKE_CASE
/// 2. Normalizes relation type names to lower_snake_case
/// 3. Deduplicates types by normalized name (keeps highest frequency)
/// 4. Ensures baseline entity types (PERSON, ORGANIZATION, LOCATION, DATE) are present
/// 5. Merges user-specified entity/relationship types (guaranteed in output)
/// 6. Sets metadata fields (status, timestamps, sampling metadata)
pub fn normalize_proposal(
    raw: RawSchemaProposal,
    sample_size: usize,
    total_documents: usize,
    domain_hint: Option<String>,
    sampling_metadata: Option<SamplingMetadata>,
    suggest_input: Option<&SuggestSchemaInput>,
) -> SchemaProposal {
    // Normalize and deduplicate entity types
    let mut entity_types = normalize_entity_types(raw.entity_types);

    // Normalize and deduplicate relation types
    let mut relation_types = normalize_relation_types(raw.relation_types);

    // Merge user-specified entity types (guaranteed in output)
    if let Some(input) = suggest_input {
        if let Some(ref expected) = input.expected_entity_types {
            for user_type in expected {
                let normalized = to_upper_snake_case(user_type);
                if !normalized.is_empty()
                    && !entity_types.iter().any(|e| e.name == normalized)
                {
                    entity_types.push(EntityTypeProposal {
                        name: normalized,
                        description: "User-specified entity type".to_string(),
                        frequency: 0,
                        is_baseline: false,
                    });
                }
            }
        }
        if let Some(ref expected) = input.expected_relationship_types {
            for user_type in expected {
                let normalized = to_lower_snake_case(user_type);
                if !normalized.is_empty()
                    && !relation_types.iter().any(|r| r.name == normalized)
                {
                    relation_types.push(RelationTypeProposal {
                        name: normalized,
                        description: "User-specified relationship type".to_string(),
                        source_type: "UNKNOWN".to_string(),
                        target_type: "UNKNOWN".to_string(),
                        frequency: 0,
                    });
                }
            }
        }
    }

    SchemaProposal {
        status: SchemaStatus::Proposed,
        entity_types,
        relation_types,
        sample_size,
        total_documents,
        domain_hint,
        proposed_at: chrono::Utc::now().timestamp_millis(),
        reviewed_at: None,
        sampling_metadata,
    }
}

/// Normalize entity type names to UPPER_SNAKE_CASE and deduplicate.
///
/// Ensures all baseline types are present.
fn normalize_entity_types(
    raw_types: Vec<crate::analyzer::RawEntityType>,
) -> Vec<EntityTypeProposal> {
    let mut by_name: HashMap<String, EntityTypeProposal> = HashMap::new();

    for raw in raw_types {
        let normalized_name = to_upper_snake_case(&raw.name);
        if normalized_name.is_empty() {
            continue;
        }

        let is_baseline = BASELINE_ENTITY_TYPES
            .iter()
            .any(|(name, _)| *name == normalized_name);

        let entry = by_name.entry(normalized_name.clone()).or_insert_with(|| {
            EntityTypeProposal {
                name: normalized_name.clone(),
                description: raw.description.clone(),
                frequency: 0,
                is_baseline,
            }
        });

        // Keep higher frequency, merge descriptions if different
        if raw.frequency > entry.frequency {
            entry.frequency = raw.frequency;
            if !raw.description.is_empty() && raw.description != entry.description {
                entry.description = raw.description;
            }
        }
    }

    // Ensure baseline types are present
    for (name, description) in BASELINE_ENTITY_TYPES {
        by_name.entry(name.to_string()).or_insert_with(|| {
            EntityTypeProposal {
                name: name.to_string(),
                description: description.to_string(),
                frequency: 0,
                is_baseline: true,
            }
        });
    }

    // Sort: baseline types first (in order), then domain types by frequency desc
    let mut baseline: Vec<EntityTypeProposal> = Vec::new();
    let mut domain: Vec<EntityTypeProposal> = Vec::new();

    for (_, proposal) in by_name {
        if proposal.is_baseline {
            baseline.push(proposal);
        } else {
            domain.push(proposal);
        }
    }

    // Sort baseline in canonical order
    baseline.sort_by(|a, b| {
        let a_idx = BASELINE_ENTITY_TYPES
            .iter()
            .position(|(n, _)| *n == a.name)
            .unwrap_or(usize::MAX);
        let b_idx = BASELINE_ENTITY_TYPES
            .iter()
            .position(|(n, _)| *n == b.name)
            .unwrap_or(usize::MAX);
        a_idx.cmp(&b_idx)
    });

    // Sort domain types by frequency descending
    domain.sort_by(|a, b| b.frequency.cmp(&a.frequency));

    let mut result = baseline;
    result.extend(domain);
    result
}

/// Normalize relation type names to lower_snake_case and deduplicate.
fn normalize_relation_types(
    raw_types: Vec<crate::analyzer::RawRelationType>,
) -> Vec<RelationTypeProposal> {
    let mut by_key: HashMap<String, RelationTypeProposal> = HashMap::new();

    for raw in raw_types {
        let normalized_name = to_lower_snake_case(&raw.name);
        if normalized_name.is_empty() {
            continue;
        }

        let normalized_source = to_upper_snake_case(&raw.source_type);
        let normalized_target = to_upper_snake_case(&raw.target_type);

        // Dedup key: name + source + target
        let key = format!("{}__{}__{}", normalized_name, normalized_source, normalized_target);

        let entry = by_key.entry(key).or_insert_with(|| RelationTypeProposal {
            name: normalized_name.clone(),
            description: raw.description.clone(),
            source_type: normalized_source,
            target_type: normalized_target,
            frequency: 0,
        });

        if raw.frequency > entry.frequency {
            entry.frequency = raw.frequency;
            if !raw.description.is_empty() && raw.description != entry.description {
                entry.description = raw.description;
            }
        }
    }

    let mut result: Vec<RelationTypeProposal> = by_key.into_values().collect();
    result.sort_by(|a, b| b.frequency.cmp(&a.frequency));
    result
}

/// Convert a string to UPPER_SNAKE_CASE.
///
/// - Uppercase all characters
/// - Replace spaces, hyphens, and dots with underscores
/// - Collapse consecutive underscores
fn to_upper_snake_case(s: &str) -> String {
    let result: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_uppercase().next().unwrap_or(c)
            } else {
                '_'
            }
        })
        .collect();

    // Collapse consecutive underscores and trim
    collapse_underscores(&result)
}

/// Convert a string to lower_snake_case.
///
/// - Lowercase all characters
/// - Replace spaces, hyphens, and dots with underscores
/// - Collapse consecutive underscores
fn to_lower_snake_case(s: &str) -> String {
    let result: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_lowercase().next().unwrap_or(c)
            } else {
                '_'
            }
        })
        .collect();

    collapse_underscores(&result)
}

/// Collapse consecutive underscores and trim leading/trailing underscores.
fn collapse_underscores(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_underscore = true; // Start true to trim leading underscores

    for c in s.chars() {
        if c == '_' {
            if !prev_underscore {
                result.push('_');
            }
            prev_underscore = true;
        } else {
            result.push(c);
            prev_underscore = false;
        }
    }

    // Trim trailing underscore
    if result.ends_with('_') {
        result.pop();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::{RawEntityType, RawRelationType, RawSchemaProposal};

    #[test]
    fn test_to_upper_snake_case() {
        assert_eq!(to_upper_snake_case("Person"), "PERSON");
        assert_eq!(to_upper_snake_case("legal case"), "LEGAL_CASE");
        assert_eq!(to_upper_snake_case("LEGAL_CASE"), "LEGAL_CASE");
        assert_eq!(to_upper_snake_case("medical-procedure"), "MEDICAL_PROCEDURE");
        assert_eq!(to_upper_snake_case("  spaces  "), "SPACES");
        assert_eq!(to_upper_snake_case("multiple___underscores"), "MULTIPLE_UNDERSCORES");
    }

    #[test]
    fn test_to_lower_snake_case() {
        assert_eq!(to_lower_snake_case("employs"), "employs");
        assert_eq!(to_lower_snake_case("filed in"), "filed_in");
        assert_eq!(to_lower_snake_case("FILED_IN"), "filed_in");
        assert_eq!(to_lower_snake_case("diagnosed-with"), "diagnosed_with");
    }

    #[test]
    fn test_baseline_types_always_present() {
        let raw = RawSchemaProposal {
            entity_types: vec![RawEntityType {
                name: "CUSTOM_TYPE".to_string(),
                description: "Custom".to_string(),
                frequency: 5,
            }],
            relation_types: vec![],
        };

        let proposal = normalize_proposal(raw, 10, 100, None, None, None);

        // Should have PERSON, ORGANIZATION, LOCATION, DATE + CUSTOM_TYPE
        assert_eq!(proposal.entity_types.len(), 5);
        assert!(proposal.entity_types.iter().any(|e| e.name == "PERSON" && e.is_baseline));
        assert!(proposal.entity_types.iter().any(|e| e.name == "ORGANIZATION" && e.is_baseline));
        assert!(proposal.entity_types.iter().any(|e| e.name == "LOCATION" && e.is_baseline));
        assert!(proposal.entity_types.iter().any(|e| e.name == "DATE" && e.is_baseline));
        assert!(proposal.entity_types.iter().any(|e| e.name == "CUSTOM_TYPE" && !e.is_baseline));
    }

    #[test]
    fn test_baseline_types_marked_when_from_llm() {
        let raw = RawSchemaProposal {
            entity_types: vec![
                RawEntityType {
                    name: "Person".to_string(),
                    description: "People in docs".to_string(),
                    frequency: 18,
                },
                RawEntityType {
                    name: "LEGAL_CASE".to_string(),
                    description: "Legal cases".to_string(),
                    frequency: 10,
                },
            ],
            relation_types: vec![],
        };

        let proposal = normalize_proposal(raw, 20, 200, None, None, None);

        let person = proposal.entity_types.iter().find(|e| e.name == "PERSON").unwrap();
        assert!(person.is_baseline);
        assert_eq!(person.frequency, 18);

        let legal = proposal.entity_types.iter().find(|e| e.name == "LEGAL_CASE").unwrap();
        assert!(!legal.is_baseline);
    }

    #[test]
    fn test_dedup_entity_types() {
        let raw = RawSchemaProposal {
            entity_types: vec![
                RawEntityType {
                    name: "Person".to_string(),
                    description: "Named individuals".to_string(),
                    frequency: 10,
                },
                RawEntityType {
                    name: "PERSON".to_string(),
                    description: "People".to_string(),
                    frequency: 18,
                },
            ],
            relation_types: vec![],
        };

        let proposal = normalize_proposal(raw, 20, 200, None, None, None);

        let persons: Vec<_> = proposal.entity_types.iter().filter(|e| e.name == "PERSON").collect();
        assert_eq!(persons.len(), 1);
        assert_eq!(persons[0].frequency, 18); // Higher frequency kept
    }

    #[test]
    fn test_dedup_relation_types() {
        let raw = RawSchemaProposal {
            entity_types: vec![],
            relation_types: vec![
                RawRelationType {
                    name: "employs".to_string(),
                    description: "Employment".to_string(),
                    source_type: "ORGANIZATION".to_string(),
                    target_type: "PERSON".to_string(),
                    frequency: 5,
                },
                RawRelationType {
                    name: "Employs".to_string(),
                    description: "Employs people".to_string(),
                    source_type: "Organization".to_string(),
                    target_type: "Person".to_string(),
                    frequency: 8,
                },
            ],
        };

        let proposal = normalize_proposal(raw, 20, 200, None, None, None);

        let employs: Vec<_> = proposal.relation_types.iter().filter(|r| r.name == "employs").collect();
        assert_eq!(employs.len(), 1);
        assert_eq!(employs[0].frequency, 8);
    }

    #[test]
    fn test_normalize_sets_metadata() {
        let raw = RawSchemaProposal {
            entity_types: vec![],
            relation_types: vec![],
        };

        let proposal = normalize_proposal(raw, 15, 150, Some("legal".to_string()), None, None);

        assert_eq!(proposal.status, SchemaStatus::Proposed);
        assert_eq!(proposal.sample_size, 15);
        assert_eq!(proposal.total_documents, 150);
        assert_eq!(proposal.domain_hint, Some("legal".to_string()));
        assert!(proposal.proposed_at > 0);
        assert!(proposal.reviewed_at.is_none());
    }

    #[test]
    fn test_baseline_order() {
        let raw = RawSchemaProposal {
            entity_types: vec![
                RawEntityType {
                    name: "CUSTOM_A".to_string(),
                    description: "A".to_string(),
                    frequency: 20,
                },
                RawEntityType {
                    name: "DATE".to_string(),
                    description: "Dates".to_string(),
                    frequency: 5,
                },
            ],
            relation_types: vec![],
        };

        let proposal = normalize_proposal(raw, 10, 100, None, None, None);

        // Baseline types should come first in canonical order
        assert_eq!(proposal.entity_types[0].name, "PERSON");
        assert_eq!(proposal.entity_types[1].name, "ORGANIZATION");
        assert_eq!(proposal.entity_types[2].name, "LOCATION");
        assert_eq!(proposal.entity_types[3].name, "DATE");
        // Then domain types
        assert_eq!(proposal.entity_types[4].name, "CUSTOM_A");
    }

    #[test]
    fn test_user_specified_entity_types_guaranteed() {
        let raw = RawSchemaProposal {
            entity_types: vec![RawEntityType {
                name: "PERSON".to_string(),
                description: "People".to_string(),
                frequency: 10,
            }],
            relation_types: vec![],
        };

        let input = SuggestSchemaInput {
            expected_entity_types: Some(vec![
                "VEHICLE".to_string(),
                "PERSON".to_string(), // Already present, should not duplicate
            ]),
            ..Default::default()
        };

        let proposal = normalize_proposal(raw, 10, 100, None, None, Some(&input));

        // VEHICLE should be added, PERSON should not be duplicated
        assert!(proposal.entity_types.iter().any(|e| e.name == "VEHICLE"));
        let persons: Vec<_> = proposal
            .entity_types
            .iter()
            .filter(|e| e.name == "PERSON")
            .collect();
        assert_eq!(persons.len(), 1);
    }

    #[test]
    fn test_user_specified_relationship_types_guaranteed() {
        let raw = RawSchemaProposal {
            entity_types: vec![],
            relation_types: vec![],
        };

        let input = SuggestSchemaInput {
            expected_relationship_types: Some(vec!["funds".to_string(), "manages".to_string()]),
            ..Default::default()
        };

        let proposal = normalize_proposal(raw, 10, 100, None, None, Some(&input));

        assert!(proposal.relation_types.iter().any(|r| r.name == "funds"));
        assert!(proposal.relation_types.iter().any(|r| r.name == "manages"));

        let funds = proposal
            .relation_types
            .iter()
            .find(|r| r.name == "funds")
            .unwrap();
        assert_eq!(funds.source_type, "UNKNOWN");
        assert_eq!(funds.target_type, "UNKNOWN");
        assert_eq!(funds.frequency, 0);
    }

    #[test]
    fn test_sampling_metadata_stored() {
        let raw = RawSchemaProposal {
            entity_types: vec![],
            relation_types: vec![],
        };

        let meta = SamplingMetadata {
            total_documents: 100,
            sampled_count: 24,
            buckets: crate::types::BucketBreakdown {
                short: crate::types::BucketInfo {
                    available: 30,
                    sampled: 8,
                },
                medium: crate::types::BucketInfo {
                    available: 50,
                    sampled: 8,
                },
                long: crate::types::BucketInfo {
                    available: 20,
                    sampled: 8,
                },
            },
            topic_clusters_found: 5,
            auto_persona: None,
        };

        let proposal = normalize_proposal(raw, 24, 100, None, Some(meta), None);

        assert!(proposal.sampling_metadata.is_some());
        let m = proposal.sampling_metadata.unwrap();
        assert_eq!(m.total_documents, 100);
        assert_eq!(m.sampled_count, 24);
        assert_eq!(m.topic_clusters_found, 5);
    }
}
