//! Namespace config resolver for the batch CLI.
//!
//! Encapsulates the full config resolution flow:
//! 1. If `--config-file` provided: load TOML, skip DynamoDB
//! 2. If no `--config-file`: fetch from DynamoDB namespace registry
//! 3. Apply CLI overrides with warnings
//! 4. Validate all config upfront and report all errors at once
//!
//! This module is the bridge between the CLI args and the pipeline config,
//! replacing the old hardcoded Epstein defaults with dynamic namespace config.

use crate::cli::Cli;
use crate::config_file::PipelineConfigFile;
use crate::domain_config::{DomainConfig, DomainMetadata, PromptConfig, RelationshipKeyword};
use edgequake_core::schema::SchemaProposal;
use edgequake_core::{NamespaceSlug, PipelineConfig, SchemaStatus};
use edgequake_storage_aws::{DynamoNamespaceConfig, DynamoNamespaceRegistry};
use indexmap::IndexMap;
use tracing::{debug, info, warn};

/// Default extraction model when namespace config has no explicit value.
const DEFAULT_EXTRACTION_MODEL: &str = "gpt-4o-mini";

/// Default embedding model when namespace config has no explicit value.
const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-3-small";

/// Default embedding dimension.
const DEFAULT_EMBEDDING_DIMENSION: u32 = 1536;

/// Resolved pipeline configuration produced by the namespace resolver.
///
/// Contains all the information needed to run the pipeline: model names,
/// embedding config, domain config for prompt generation, and a summary
/// string for startup logging.
pub struct ResolvedConfig {
    /// The LLM model to use for entity extraction.
    pub extraction_model: String,

    /// The embedding model to use for vector generation.
    pub embedding_model: String,

    /// The embedding vector dimension.
    pub embedding_dimension: u32,

    /// Domain configuration for extraction prompt generation.
    pub domain_config: DomainConfig,

    /// Human-readable summary for startup logging.
    /// e.g., "5 entity types, 3 relation types from schema 'legal-financial'"
    pub schema_summary: String,
}

/// Resolves pipeline configuration from DynamoDB namespace registry or TOML config file.
///
/// Created with the namespace slug and DynamoDB client, then called with `resolve()`
/// to produce a `ResolvedConfig` that the pipeline can consume.
pub struct NamespaceConfigResolver {
    namespace: NamespaceSlug,
    aws_config: aws_config::SdkConfig,
    registry_table: String,
}

impl NamespaceConfigResolver {
    /// Create a new resolver for the given namespace.
    ///
    /// # Arguments
    ///
    /// * `namespace` - Validated namespace slug
    /// * `aws_config` - AWS SDK config for creating clients
    /// * `registry_table` - DynamoDB namespace registry table (PK + SK schema)
    pub fn new(
        namespace: NamespaceSlug,
        aws_config: aws_config::SdkConfig,
        registry_table: String,
    ) -> Self {
        Self {
            namespace,
            aws_config,
            registry_table,
        }
    }

    /// Resolve the full pipeline configuration.
    ///
    /// Resolution order:
    /// 1. If `--config-file` provided: load TOML, skip DynamoDB
    /// 2. Otherwise: fetch from DynamoDB namespace registry
    /// 3. Apply CLI overrides (`--model`, `--embedding-model`) with warnings
    /// 4. Resolve domain config from schema or `--domain-config`
    /// 5. Validate everything upfront
    pub async fn resolve(&self, cli: &Cli) -> anyhow::Result<ResolvedConfig> {
        if cli.config_file.is_some() {
            self.resolve_from_config_file(cli).await
        } else {
            self.resolve_from_dynamodb(cli).await
        }
    }

    /// Resolve config from a TOML config file (skips DynamoDB entirely).
    async fn resolve_from_config_file(&self, cli: &Cli) -> anyhow::Result<ResolvedConfig> {
        let config_path = cli.config_file.as_ref().unwrap();
        info!(path = %config_path.display(), "Loading pipeline config from file (DynamoDB lookup skipped)");

        let config_file = PipelineConfigFile::load(config_path)?;

        // Resolve models: CLI override > config file > defaults
        let mut extraction_model = config_file
            .pipeline
            .llm_model
            .unwrap_or_else(|| DEFAULT_EXTRACTION_MODEL.to_string());
        let mut embedding_model = config_file
            .pipeline
            .embedding_model
            .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_string());
        let embedding_dimension = config_file
            .pipeline
            .embedding_dimension
            .unwrap_or(DEFAULT_EMBEDDING_DIMENSION);

        // Apply CLI overrides with warnings
        if let Some(ref cli_model) = cli.model {
            warn!(
                "Overriding extraction model from config file '{}' -> '{}'. This may cause misalignment.",
                extraction_model, cli_model
            );
            extraction_model = cli_model.clone();
        }
        if let Some(ref cli_embed) = cli.embedding_model {
            warn!(
                "Overriding embedding model from config file '{}' -> '{}'. This may cause misalignment.",
                embedding_model, cli_embed
            );
            embedding_model = cli_embed.clone();
        }

        // Resolve domain config
        let (domain_config, schema_summary) = if let Some(ref domain_path) = cli.domain_config {
            // --domain-config fully replaces schema-derived prompts
            let dc = DomainConfig::from_file(domain_path)?;
            let summary = format!(
                "{} entity types from domain config '{}'",
                dc.entity_types.len(),
                domain_path.display()
            );
            (dc, summary)
        } else if let Some(ref schema) = config_file.schema {
            // Build DomainConfig from config file's [schema] section
            let dc = build_domain_config_from_config_schema(schema, self.namespace.as_str());
            let summary = format!(
                "{} entity types, {} relation types from config file",
                schema.entity_types.len(),
                schema.relation_types.len()
            );
            (dc, summary)
        } else {
            anyhow::bail!(
                "Config file '{}' has no [schema] section and no --domain-config provided. \
                 Either add a [schema] section to the config file or provide --domain-config.",
                config_path.display()
            );
        };

        Ok(ResolvedConfig {
            extraction_model,
            embedding_model,
            embedding_dimension,
            domain_config,
            schema_summary,
        })
    }

    /// Resolve config from DynamoDB namespace registry.
    async fn resolve_from_dynamodb(&self, cli: &Cli) -> anyhow::Result<ResolvedConfig> {
        let dynamo_client = aws_sdk_dynamodb::Client::new(&self.aws_config);
        let registry_config = DynamoNamespaceConfig {
            table_name: self.registry_table.clone(),
        };
        let registry = DynamoNamespaceRegistry::new(registry_config, dynamo_client);

        let mut errors: Vec<String> = Vec::new();

        // 1. Check namespace exists
        let _ns_record = match registry.describe_namespace(&self.namespace).await {
            Ok(Some(record)) => {
                debug!(namespace = %self.namespace, "Found namespace record");
                record
            }
            Ok(None) => {
                let (account_id, region) = self.get_aws_identity().await;
                let mut msg = format!(
                    "Namespace '{}' not found in {} {}. Please create and configure it first.",
                    self.namespace, account_id, region
                );
                // Hint at UI URL if available
                if let Ok(ui_url) = std::env::var("EDGEQUAKE_UI_URL") {
                    msg.push_str(&format!(" Configure namespaces at: {}", ui_url));
                }
                errors.push(msg);
                // Can't continue without namespace
                anyhow::bail!("Configuration errors:\n{}", errors.join("\n"));
            }
            Err(e) => {
                anyhow::bail!("Failed to check namespace '{}': {}", self.namespace, e);
            }
        };

        // 2. Get pipeline config
        let pipeline_config = match registry.get_config(&self.namespace).await {
            Ok(Some(config)) => config,
            Ok(None) => {
                warn!(
                    namespace = %self.namespace,
                    "No pipeline config found, using defaults"
                );
                PipelineConfig::default()
            }
            Err(e) => {
                anyhow::bail!(
                    "Failed to read config for namespace '{}': {}",
                    self.namespace,
                    e
                );
            }
        };

        // Resolve models: CLI override > DynamoDB config > defaults
        let mut extraction_model = if pipeline_config.llm_model.is_empty() {
            warn!(
                "No extraction model configured for namespace '{}', using default '{}'",
                self.namespace, DEFAULT_EXTRACTION_MODEL
            );
            DEFAULT_EXTRACTION_MODEL.to_string()
        } else {
            pipeline_config.llm_model.clone()
        };

        let mut embedding_model = if pipeline_config.embedding_model.is_empty() {
            warn!(
                "No embedding model configured for namespace '{}', using default '{}'",
                self.namespace, DEFAULT_EMBEDDING_MODEL
            );
            DEFAULT_EMBEDDING_MODEL.to_string()
        } else {
            pipeline_config.embedding_model.clone()
        };

        let embedding_dimension = if pipeline_config.embedding_dimension == 0 {
            DEFAULT_EMBEDDING_DIMENSION
        } else {
            pipeline_config.embedding_dimension as u32
        };

        // Apply CLI overrides with warnings
        if let Some(ref cli_model) = cli.model {
            warn!(
                "Overriding extraction model from namespace config '{}' -> '{}'. This may cause misalignment.",
                extraction_model, cli_model
            );
            extraction_model = cli_model.clone();
        }
        if let Some(ref cli_embed) = cli.embedding_model {
            warn!(
                "Overriding embedding model from namespace config '{}' -> '{}'. This may cause misalignment.",
                embedding_model, cli_embed
            );
            embedding_model = cli_embed.clone();
        }

        // 3. Resolve domain config from schema or --domain-config
        let (domain_config, schema_summary) = if let Some(ref domain_path) = cli.domain_config {
            // --domain-config fully replaces schema-derived prompts, bypasses schema requirement
            info!(
                path = %domain_path.display(),
                "Using --domain-config (bypassing schema requirement)"
            );
            let dc = DomainConfig::from_file(domain_path)?;
            let summary = format!(
                "{} entity types from domain config '{}'",
                dc.entity_types.len(),
                domain_path.display()
            );
            (dc, summary)
        } else {
            // Need an approved schema from DynamoDB
            match registry.get_schema(&self.namespace).await {
                Ok(Some(proposal)) => match proposal.status {
                    SchemaStatus::Approved => {
                        let dc =
                            build_domain_config_from_schema(&proposal, self.namespace.as_str());
                        let summary = format!(
                            "{} entity types, {} relation types from schema '{}'",
                            proposal.entity_types.len(),
                            proposal.relation_types.len(),
                            self.namespace
                        );
                        (dc, summary)
                    }
                    SchemaStatus::Proposed => {
                        errors.push(format!(
                            "Schema has been proposed but not yet approved for namespace '{}'. \
                             Review and approve the schema before running ingestion.",
                            self.namespace
                        ));
                        // Fall through to error reporting
                        let dc = DomainConfig::placeholder();
                        (dc, String::new())
                    }
                    SchemaStatus::Rejected => {
                        errors.push(format!(
                            "Schema was rejected for namespace '{}'. \
                             Run schema suggestion again or approve the existing proposal.",
                            self.namespace
                        ));
                        let dc = DomainConfig::placeholder();
                        (dc, String::new())
                    }
                    SchemaStatus::None => {
                        errors.push(format!(
                            "Namespace '{}' has no approved schema. \
                             Approve a schema in the UI or provide --domain-config.",
                            self.namespace
                        ));
                        let dc = DomainConfig::placeholder();
                        (dc, String::new())
                    }
                },
                Ok(None) => {
                    errors.push(format!(
                        "Namespace '{}' has no approved schema. \
                         Approve a schema in the UI or provide --domain-config.",
                        self.namespace
                    ));
                    let dc = DomainConfig::placeholder();
                    (dc, String::new())
                }
                Err(e) => {
                    anyhow::bail!(
                        "Failed to read schema for namespace '{}': {}",
                        self.namespace,
                        e
                    );
                }
            }
        };

        // 4. Report all collected errors at once
        if !errors.is_empty() {
            anyhow::bail!("Configuration errors:\n{}", errors.join("\n"));
        }

        Ok(ResolvedConfig {
            extraction_model,
            embedding_model,
            embedding_dimension,
            domain_config,
            schema_summary,
        })
    }

    /// Get AWS account ID and region for error messages.
    ///
    /// Best-effort: if STS call fails, returns placeholder strings.
    async fn get_aws_identity(&self) -> (String, String) {
        let region = self
            .aws_config
            .region()
            .map(|r| r.to_string())
            .unwrap_or_else(|| "<unknown-region>".to_string());

        let sts_client = aws_sdk_sts::Client::new(&self.aws_config);
        let account_id = match sts_client.get_caller_identity().send().await {
            Ok(identity) => identity
                .account()
                .unwrap_or("<unknown-account>")
                .to_string(),
            Err(e) => {
                debug!(error = %e, "STS get_caller_identity failed, using placeholder");
                "<unknown-account>".to_string()
            }
        };

        (account_id, region)
    }
}

/// Build a `DomainConfig` from an approved `SchemaProposal`.
///
/// Uses the schema's entity types (with descriptions) and relation types
/// to construct extraction prompts. This is the primary path for
/// schema-driven domain-agnostic extraction.
fn build_domain_config_from_schema(
    proposal: &SchemaProposal,
    namespace_name: &str,
) -> DomainConfig {
    let mut entity_types = IndexMap::new();
    for et in &proposal.entity_types {
        entity_types.insert(et.name.clone(), et.description.clone());
    }

    let mut relationship_keywords = IndexMap::new();
    let keywords: Vec<RelationshipKeyword> = proposal
        .relation_types
        .iter()
        .map(|rt| RelationshipKeyword {
            keyword: rt.name.clone(),
            description: rt.description.clone(),
        })
        .collect();
    if !keywords.is_empty() {
        relationship_keywords.insert("domain".to_string(), keywords);
    }

    DomainConfig {
        domain: DomainMetadata {
            name: namespace_name.to_string(),
            description: format!("Schema-derived config for namespace '{}'", namespace_name),
            language: "English".to_string(),
        },
        entity_types,
        aliases: IndexMap::new(),
        prompts: PromptConfig {
            role_description: format!(
                "You are a Knowledge Graph Specialist responsible for extracting \
                 entities and relationships from documents in the '{}' domain.",
                namespace_name
            ),
            canonicalization_examples: vec![],
            extra_instructions: vec![],
            user_instructions: vec![],
        },
        relationship_keywords,
        examples: vec![],
    }
}

/// Build a `DomainConfig` from a config file's `[schema]` section.
///
/// Similar to `build_domain_config_from_schema` but uses the config file
/// schema types instead of a `SchemaProposal`.
fn build_domain_config_from_config_schema(
    schema: &crate::config_file::SchemaSection,
    namespace_name: &str,
) -> DomainConfig {
    let mut entity_types = IndexMap::new();
    for et in &schema.entity_types {
        entity_types.insert(et.name.clone(), et.description.clone());
    }

    let mut relationship_keywords = IndexMap::new();
    let keywords: Vec<RelationshipKeyword> = schema
        .relation_types
        .iter()
        .map(|rt| RelationshipKeyword {
            keyword: rt.name.clone(),
            description: rt.description.clone(),
        })
        .collect();
    if !keywords.is_empty() {
        relationship_keywords.insert("domain".to_string(), keywords);
    }

    DomainConfig {
        domain: DomainMetadata {
            name: namespace_name.to_string(),
            description: format!("Config file schema for namespace '{}'", namespace_name),
            language: "English".to_string(),
        },
        entity_types,
        aliases: IndexMap::new(),
        prompts: PromptConfig {
            role_description: format!(
                "You are a Knowledge Graph Specialist responsible for extracting \
                 entities and relationships from documents in the '{}' domain.",
                namespace_name
            ),
            canonicalization_examples: vec![],
            extra_instructions: vec![],
            user_instructions: vec![],
        },
        relationship_keywords,
        examples: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use edgequake_core::schema::{
        EntityTypeProposal, RelationTypeProposal, SchemaProposal, SchemaStatus,
    };

    fn sample_proposal() -> SchemaProposal {
        SchemaProposal {
            status: SchemaStatus::Approved,
            entity_types: vec![
                EntityTypeProposal {
                    name: "PERSON".to_string(),
                    description: "Named individuals".to_string(),
                    frequency: 10,
                    is_baseline: true,
                },
                EntityTypeProposal {
                    name: "ORGANIZATION".to_string(),
                    description: "Companies and agencies".to_string(),
                    frequency: 8,
                    is_baseline: true,
                },
            ],
            relation_types: vec![RelationTypeProposal {
                name: "employs".to_string(),
                description: "Employment relationship".to_string(),
                source_type: "ORGANIZATION".to_string(),
                target_type: "PERSON".to_string(),
                frequency: 5,
            }],
            sample_size: 10,
            total_documents: 100,
            domain_hint: Some("test".to_string()),
            proposed_at: 1700000000000,
            reviewed_at: Some(1700000001000),
            sampling_metadata: None,
        }
    }

    #[test]
    fn test_build_domain_config_from_schema() {
        let proposal = sample_proposal();
        let dc = build_domain_config_from_schema(&proposal, "test-ns");

        assert_eq!(dc.domain.name, "test-ns");
        assert_eq!(dc.entity_types.len(), 2);
        assert!(dc.entity_types.contains_key("PERSON"));
        assert!(dc.entity_types.contains_key("ORGANIZATION"));
        assert_eq!(dc.entity_types["PERSON"], "Named individuals");

        // Check relationship keywords
        let domain_keywords = dc.relationship_keywords.get("domain").unwrap();
        assert_eq!(domain_keywords.len(), 1);
        assert_eq!(domain_keywords[0].keyword, "employs");
        assert_eq!(domain_keywords[0].description, "Employment relationship");

        // Validate
        dc.validate()
            .expect("schema-derived config should be valid");
    }

    #[test]
    fn test_build_domain_config_from_config_schema() {
        use crate::config_file::{SchemaEntityType, SchemaRelationType, SchemaSection};

        let schema = SchemaSection {
            entity_types: vec![SchemaEntityType {
                name: "DOCUMENT".to_string(),
                description: "Legal documents".to_string(),
            }],
            relation_types: vec![SchemaRelationType {
                name: "references".to_string(),
                description: "Document references another".to_string(),
                source_type: Some("DOCUMENT".to_string()),
                target_type: Some("DOCUMENT".to_string()),
            }],
        };

        let dc = build_domain_config_from_config_schema(&schema, "doc-ns");
        assert_eq!(dc.domain.name, "doc-ns");
        assert_eq!(dc.entity_types.len(), 1);
        assert!(dc.entity_types.contains_key("DOCUMENT"));
        dc.validate()
            .expect("config-schema-derived config should be valid");
    }

    #[test]
    fn test_placeholder_domain_config() {
        let dc = DomainConfig::placeholder();
        // Should have at least one entity type to pass validation
        assert!(!dc.entity_types.is_empty());
    }
}
