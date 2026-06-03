//! Domain-specific configuration for extraction pipelines.
//!
//! Loads entity types, aliases, prompt content, and few-shot examples from a TOML file
//! or generates them from an approved schema proposal, allowing extraction pipelines to
//! be used for any domain without code changes.
//!
//! ## Config Sources
//!
//! - `DomainConfig::from_schema()` — generated from an approved namespace schema
//! - `DomainConfig::from_file()` — loaded from a TOML file (`--domain-config` escape hatch)
//! - `DomainConfig::load()` — resolves from CLI path or returns an error

use crate::schema::SchemaProposal;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::info;

/// Root domain configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainConfig {
    /// Domain metadata.
    pub domain: DomainMetadata,

    /// Entity types: type_name -> description (ordered).
    pub entity_types: IndexMap<String, String>,

    /// Alias mappings: canonical_name -> [aliases].
    #[serde(default)]
    pub aliases: IndexMap<String, Vec<String>>,

    /// Prompt configuration.
    pub prompts: PromptConfig,

    /// Relationship keywords by category.
    #[serde(default)]
    pub relationship_keywords: IndexMap<String, Vec<RelationshipKeyword>>,

    /// Few-shot examples.
    #[serde(default)]
    pub examples: Vec<FewShotExample>,
}

/// Domain metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainMetadata {
    /// Short name for this domain (e.g. "legal-financial", "medical").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Output language (default: "English").
    #[serde(default = "default_language")]
    pub language: String,
}

fn default_language() -> String {
    "English".to_string()
}

/// Prompt configuration: role, canonicalization hints, and extra instruction sections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptConfig {
    /// Role description paragraph for the system prompt.
    pub role_description: String,

    /// Canonicalization examples for entity names (e.g. `"Virginia Giuffre" (NOT "Virginia Roberts")`).
    #[serde(default)]
    pub canonicalization_examples: Vec<String>,

    /// Extra instruction sections appended to the system prompt (e.g. timestamp, email instructions).
    #[serde(default)]
    pub extra_instructions: Vec<InstructionSection>,

    /// Extra lines added to the user prompt instructions list.
    #[serde(default)]
    pub user_instructions: Vec<UserInstruction>,
}

/// A titled section of extra instructions for the system prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionSection {
    /// Section title (e.g. "Timestamp Instructions").
    pub title: String,
    /// Section content.
    pub content: String,
}

/// An extra line for the user prompt instructions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInstruction {
    /// Instruction content.
    pub content: String,
}

/// A relationship keyword with description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipKeyword {
    /// The keyword (e.g. "financial_transaction").
    pub keyword: String,
    /// Description (e.g. "payments, wire transfers").
    pub description: String,
}

/// A few-shot example for the system prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FewShotExample {
    /// Title (e.g. "Email", "Legal document").
    pub title: String,
    /// Input text.
    pub input: String,
    /// Expected output (may contain `{td}` and `{cd}` placeholders).
    pub output: String,
}

impl DomainConfig {
    /// Load domain configuration from an explicit TOML file path.
    ///
    /// If `cli_path` is `Some`, loads from that file. If `None`, returns an error
    /// directing the user to either approve a schema or provide `--domain-config`.
    pub fn load(cli_path: Option<&Path>) -> anyhow::Result<Self> {
        if let Some(path) = cli_path {
            info!(path = %path.display(), "Loading domain config from CLI flag");
            return Self::from_file(path);
        }

        anyhow::bail!(
            "No domain config provided. Either approve a schema in the namespace \
             or provide --domain-config <path>"
        )
    }

    /// Load from a TOML file.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            anyhow::anyhow!(
                "Failed to read domain config from {}: {}",
                path.display(),
                e
            )
        })?;
        let config: Self = toml::from_str(&content).map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse domain config from {}: {}",
                path.display(),
                e
            )
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Build a `DomainConfig` from an approved `SchemaProposal`.
    ///
    /// Maps entity types (with descriptions) and relation types (as relationship
    /// keywords) from the schema proposal into the prompt configuration structure.
    pub fn from_schema(proposal: &SchemaProposal, namespace_name: &str) -> Self {
        debug_assert!(
            !proposal.entity_types.is_empty(),
            "from_schema requires non-empty entity types"
        );

        // 1. Entity types: name -> description (ordered)
        let mut entity_types = IndexMap::new();
        for et in &proposal.entity_types {
            entity_types.insert(et.name.clone(), et.description.clone());
        }

        // 2. Relationship keywords from relation types (include entity type constraints)
        let mut relationship_keywords = IndexMap::new();
        let keywords: Vec<RelationshipKeyword> = proposal
            .relation_types
            .iter()
            .map(|rt| {
                let type_hint = if !rt.source_type.is_empty() && !rt.target_type.is_empty() {
                    format!(
                        "{} — connects {} → {}",
                        rt.description, rt.source_type, rt.target_type
                    )
                } else {
                    rt.description.clone()
                };
                RelationshipKeyword {
                    keyword: rt.name.clone(),
                    description: type_hint,
                }
            })
            .collect();
        if !keywords.is_empty() {
            relationship_keywords.insert("domain".to_string(), keywords);
        }

        // 3. Domain metadata
        let domain = DomainMetadata {
            name: namespace_name.to_string(),
            description: format!("Schema-derived config for namespace '{}'", namespace_name),
            language: "English".to_string(),
        };

        // 4. Prompts with generic role description
        let prompts = PromptConfig {
            role_description: format!(
                "You are a Knowledge Graph Specialist responsible for extracting entities \
                 and relationships from documents in the '{}' domain. \
                 Your output will be used to build a structured knowledge graph where \
                 entities are nodes and relationships are typed edges. \
                 Every relationship MUST use one of the defined relationship types as its keyword. \
                 If no defined type fits a relationship, do NOT extract it.",
                namespace_name
            ),
            canonicalization_examples: vec![],
            extra_instructions: vec![],
            user_instructions: vec![],
        };

        // 5. Generate a few-shot example from the first 3 relation types
        let examples = Self::generate_schema_example(&proposal.relation_types, &proposal.entity_types);

        DomainConfig {
            domain,
            entity_types,
            aliases: IndexMap::new(),
            prompts,
            relationship_keywords,
            examples,
        }
    }

    /// Create a minimal placeholder DomainConfig for error paths.
    ///
    /// This is used when resolving config but collecting errors -- it should
    /// never be used by the actual extraction pipeline.
    pub fn placeholder() -> Self {
        let mut entity_types = IndexMap::new();
        entity_types.insert("PLACEHOLDER".to_string(), "Placeholder".to_string());
        DomainConfig {
            domain: DomainMetadata {
                name: "placeholder".to_string(),
                description: "Placeholder (should not be used)".to_string(),
                language: "English".to_string(),
            },
            entity_types,
            aliases: IndexMap::new(),
            prompts: PromptConfig {
                role_description: "Placeholder".to_string(),
                canonicalization_examples: vec![],
                extra_instructions: vec![],
                user_instructions: vec![],
            },
            relationship_keywords: IndexMap::new(),
            examples: vec![],
        }
    }

    /// Validate the configuration.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.entity_types.is_empty() {
            anyhow::bail!("domain config must define at least one entity type");
        }
        if self.prompts.role_description.is_empty() {
            anyhow::bail!("domain config must have a role_description");
        }
        Ok(())
    }

    /// Get entity type names in order.
    pub fn entity_type_names(&self) -> Vec<&str> {
        self.entity_types.keys().map(|s| s.as_str()).collect()
    }

    /// Get alias pairs as (alias, canonical) for EntityResolver.
    pub fn alias_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        for (canonical, aliases) in &self.aliases {
            for alias in aliases {
                pairs.push((alias.clone(), canonical.clone()));
            }
        }
        pairs
    }

    /// Generate a synthetic few-shot example from the schema's relation types.
    ///
    /// Picks up to 3 relation types and builds a plausible input/output pair
    /// showing entities and relationships using the exact defined keywords.
    fn generate_schema_example(
        relation_types: &[crate::schema::RelationTypeProposal],
        entity_types: &[crate::schema::EntityTypeProposal],
    ) -> Vec<FewShotExample> {
        if relation_types.is_empty() || entity_types.is_empty() {
            return vec![];
        }

        // Pick up to 3 relation types for the example
        let sample: Vec<_> = relation_types.iter().take(3).collect();

        // Build synthetic input text
        let mut input_lines = Vec::new();
        let mut output_lines = Vec::new();

        // Add entity lines and relationship lines for each sampled relation
        for rt in &sample {
            let src_name = format!("Example {}", rt.source_type.to_lowercase());
            let tgt_name = format!("Example {}", rt.target_type.to_lowercase());

            // Entities
            output_lines.push(format!(
                "entity{{td}}{}{{td}}{}{{td}}An example {} entity.",
                src_name, rt.source_type, rt.source_type.to_lowercase()
            ));
            output_lines.push(format!(
                "entity{{td}}{}{{td}}{}{{td}}An example {} entity.",
                tgt_name, rt.target_type, rt.target_type.to_lowercase()
            ));

            // Relationship using the exact keyword
            output_lines.push(format!(
                "relation{{td}}{}{{td}}{}{{td}}{}{{td}}{}",
                src_name, tgt_name, rt.name, rt.description
            ));

            input_lines.push(format!(
                "{} is connected to {} ({}).",
                src_name, tgt_name, rt.description
            ));
        }

        output_lines.push("{cd}".to_string());

        vec![FewShotExample {
            title: "Schema relationship types".to_string(),
            input: input_lines.join(" "),
            output: output_lines.join("\n"),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{
        EntityTypeProposal, RelationTypeProposal, SchemaProposal, SchemaStatus,
    };

    /// Helper: build a small test DomainConfig for unit tests.
    fn make_test_config() -> DomainConfig {
        let mut entity_types = IndexMap::new();
        entity_types.insert("PERSON".to_string(), "A named individual".to_string());
        entity_types.insert(
            "ORGANIZATION".to_string(),
            "A company or institution".to_string(),
        );

        let mut relationship_keywords = IndexMap::new();
        relationship_keywords.insert(
            "domain".to_string(),
            vec![RelationshipKeyword {
                keyword: "employs".to_string(),
                description: "Employment relationship".to_string(),
            }],
        );

        DomainConfig {
            domain: DomainMetadata {
                name: "test".to_string(),
                description: "Test domain config".to_string(),
                language: "English".to_string(),
            },
            entity_types,
            aliases: IndexMap::new(),
            prompts: PromptConfig {
                role_description: "You are a Knowledge Graph Specialist.".to_string(),
                canonicalization_examples: vec![],
                extra_instructions: vec![],
                user_instructions: vec![],
            },
            relationship_keywords,
            examples: vec![],
        }
    }

    /// Helper: build a test SchemaProposal.
    fn make_test_proposal() -> SchemaProposal {
        SchemaProposal {
            status: SchemaStatus::Approved,
            entity_types: vec![
                EntityTypeProposal {
                    name: "PERSON".to_string(),
                    description: "A named individual".to_string(),
                    frequency: 18,
                    is_baseline: true,
                },
                EntityTypeProposal {
                    name: "LEGAL_CASE".to_string(),
                    description: "A court case or legal proceeding".to_string(),
                    frequency: 10,
                    is_baseline: false,
                },
            ],
            relation_types: vec![RelationTypeProposal {
                name: "filed_in".to_string(),
                description: "Case filed in court".to_string(),
                source_type: "LEGAL_CASE".to_string(),
                target_type: "ORGANIZATION".to_string(),
                frequency: 5,
            }],
            sample_size: 20,
            total_documents: 200,
            domain_hint: Some("legal".to_string()),
            proposed_at: 1700000000000,
            reviewed_at: Some(1700000100000),
            sampling_metadata: None,
        }
    }

    #[test]
    fn test_roundtrip_toml_serialization() {
        let config = make_test_config();
        let toml_str = toml::to_string_pretty(&config).expect("should serialize to TOML");
        let parsed: DomainConfig =
            toml::from_str(&toml_str).expect("should parse back from TOML");
        assert_eq!(parsed.entity_type_names(), config.entity_type_names());
    }

    #[test]
    fn test_validate_rejects_empty_entity_types() {
        let mut config = make_test_config();
        config.entity_types.clear();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_empty_role() {
        let mut config = make_test_config();
        config.prompts.role_description.clear();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_from_schema_produces_valid_config() {
        let proposal = make_test_proposal();
        let config = DomainConfig::from_schema(&proposal, "legal-financial");

        // Entity types preserved with descriptions
        assert_eq!(config.entity_types.len(), 2);
        assert_eq!(
            config.entity_types.get("PERSON"),
            Some(&"A named individual".to_string())
        );
        assert_eq!(
            config.entity_types.get("LEGAL_CASE"),
            Some(&"A court case or legal proceeding".to_string())
        );

        // Relationship keywords from relation types
        let domain_keywords = config.relationship_keywords.get("domain").unwrap();
        assert_eq!(domain_keywords.len(), 1);
        assert_eq!(domain_keywords[0].keyword, "filed_in");
        assert_eq!(domain_keywords[0].description, "Case filed in court");

        // Domain metadata
        assert_eq!(config.domain.name, "legal-financial");
        assert!(config
            .domain
            .description
            .contains("legal-financial"));

        // Validates successfully
        config.validate().expect("from_schema config should be valid");
    }

    #[test]
    fn test_from_schema_empty_relation_types() {
        let mut proposal = make_test_proposal();
        proposal.relation_types.clear();

        let config = DomainConfig::from_schema(&proposal, "docs");

        // Entity types still present
        assert_eq!(config.entity_types.len(), 2);

        // No relationship keywords when no relation types
        assert!(config.relationship_keywords.is_empty());

        // Still valid
        config.validate().expect("config with no relations should be valid");
    }

    #[test]
    fn test_load_without_path_errors() {
        let result = DomainConfig::load(None);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("No domain config provided"),
            "Expected 'No domain config provided' error, got: {}",
            err_msg
        );
    }
}
