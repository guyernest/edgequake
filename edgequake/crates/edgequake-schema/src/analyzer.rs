//! LLM-based schema analysis using OpenAI.
//!
//! Provides two analysis modes:
//! - **Legacy single-pass** (`analyze_schema`): Single LLM call for both entity and
//!   relation types (kept for backward compatibility)
//! - **Two-pass** (`analyze_schema_two_pass`): Separate entity discovery and
//!   relationship discovery passes, with optional auto-persona generation
//!
//! The two-pass approach produces better schemas because the relationship
//! discovery pass has full context of the discovered entity types.

use async_openai::{
    config::OpenAIConfig as AsyncOpenAIConfig,
    types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs,
        ResponseFormat, ResponseFormatJsonSchema,
    },
    Client,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::prompt;
use crate::sampler::SampledDocument;
use crate::types::SuggestSchemaInput;

// ===========================================================================
// Configuration
// ===========================================================================

/// Configuration for OpenAI API access.
#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    /// OpenAI API key.
    pub api_key: String,
    /// Model to use for schema analysis (default: "gpt-4.1-mini").
    pub model: String,
    /// Optional base URL for OpenAI-compatible APIs.
    pub base_url: Option<String>,
}

impl OpenAiConfig {
    /// Create a new config with the given API key, using the default model.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: "gpt-4.1-mini".to_string(),
            base_url: None,
        }
    }

    /// Set the model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Set the base URL.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }
}

// ===========================================================================
// Raw LLM output types
// ===========================================================================

/// Raw entity type from LLM output (before normalization).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEntityType {
    pub name: String,
    pub description: String,
    pub frequency: usize,
}

/// Raw relation type from LLM output (before normalization).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawRelationType {
    pub name: String,
    pub description: String,
    pub source_type: String,
    pub target_type: String,
    pub frequency: usize,
}

/// Raw schema proposal from LLM output (before normalization).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSchemaProposal {
    pub entity_types: Vec<RawEntityType>,
    pub relation_types: Vec<RawRelationType>,
}

// ===========================================================================
// Two-pass response types
// ===========================================================================

/// Result from auto-persona generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaResult {
    pub domain: String,
    pub persona: String,
    pub key_themes: Vec<String>,
}

/// Response from entity-only discovery pass (Pass 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawEntityOnlyProposal {
    pub entity_types: Vec<RawEntityType>,
}

/// Response from relationship-only discovery pass (Pass 2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawRelationOnlyProposal {
    pub relation_types: Vec<RawRelationType>,
}

// ===========================================================================
// Legacy single-pass analysis (backward compatible)
// ===========================================================================

/// Maximum tokens to estimate before considering batching.
/// GPT-4.1-mini has 1M context; we leave buffer for prompt template + response.
const MAX_ESTIMATED_TOKENS: usize = 900_000;

/// Analyze sampled documents using OpenAI to propose a schema (legacy single-pass).
///
/// Constructs the prompt, sends it to the LLM, and parses the JSON response.
/// If the total text exceeds the context window, splits into batches and merges.
///
/// **Legacy function** — new code should use `analyze_schema_two_pass`.
pub async fn analyze_schema(
    documents: &[SampledDocument],
    domain_hint: Option<&str>,
    openai_config: &OpenAiConfig,
) -> anyhow::Result<RawSchemaProposal> {
    // Estimate total tokens (rough heuristic: 4 chars per token)
    let total_chars: usize = documents.iter().map(|d| d.content.len()).sum();
    let estimated_tokens = total_chars / 4;

    info!(
        documents = documents.len(),
        estimated_tokens = estimated_tokens,
        model = %openai_config.model,
        "Analyzing schema with LLM (single-pass)"
    );

    if estimated_tokens <= MAX_ESTIMATED_TOKENS {
        analyze_single_batch(documents, domain_hint, openai_config).await
    } else {
        info!(
            estimated_tokens = estimated_tokens,
            max_tokens = MAX_ESTIMATED_TOKENS,
            "Text exceeds context window, splitting into batches"
        );
        analyze_multi_batch(documents, domain_hint, openai_config).await
    }
}

/// Analyze a single batch of documents (legacy).
async fn analyze_single_batch(
    documents: &[SampledDocument],
    domain_hint: Option<&str>,
    openai_config: &OpenAiConfig,
) -> anyhow::Result<RawSchemaProposal> {
    let user_prompt = prompt::build_schema_prompt(documents, domain_hint);
    let json_schema = prompt::build_json_schema();

    let system_message = format!(
        "You are a schema analyst. Respond ONLY with valid JSON matching this schema:\n{}",
        serde_json::to_string_pretty(&json_schema["schema"])?
    );

    call_openai_generic::<RawSchemaProposal>(
        &system_message,
        &user_prompt,
        json_schema,
        openai_config,
        0.2,
    )
    .await
}

/// Analyze documents in multiple batches, then merge results (legacy).
async fn analyze_multi_batch(
    documents: &[SampledDocument],
    domain_hint: Option<&str>,
    openai_config: &OpenAiConfig,
) -> anyhow::Result<RawSchemaProposal> {
    let mut batches: Vec<Vec<&SampledDocument>> = Vec::new();
    let mut current_batch: Vec<&SampledDocument> = Vec::new();
    let mut current_tokens: usize = 0;
    let batch_token_limit = MAX_ESTIMATED_TOKENS;

    for doc in documents {
        let doc_tokens = doc.content.len() / 4;
        if current_tokens + doc_tokens > batch_token_limit && !current_batch.is_empty() {
            batches.push(current_batch);
            current_batch = Vec::new();
            current_tokens = 0;
        }
        current_batch.push(doc);
        current_tokens += doc_tokens;
    }
    if !current_batch.is_empty() {
        batches.push(current_batch);
    }

    info!(batch_count = batches.len(), "Processing in batches");

    let mut merged_entity_types: Vec<RawEntityType> = Vec::new();
    let mut merged_relation_types: Vec<RawRelationType> = Vec::new();

    for (i, batch) in batches.iter().enumerate() {
        info!(
            batch = i + 1,
            total = batches.len(),
            documents = batch.len(),
            "Processing batch"
        );

        let batch_docs: Vec<SampledDocument> = batch.iter().map(|d| (*d).clone()).collect();
        let result = analyze_single_batch(&batch_docs, domain_hint, openai_config).await?;

        merged_entity_types.extend(result.entity_types);
        merged_relation_types.extend(result.relation_types);
    }

    // Deduplicate by name (keep highest frequency)
    merged_entity_types.sort_by(|a, b| b.frequency.cmp(&a.frequency));
    merged_entity_types.dedup_by(|a, b| a.name.to_uppercase() == b.name.to_uppercase());

    merged_relation_types.sort_by(|a, b| b.frequency.cmp(&a.frequency));
    merged_relation_types.dedup_by(|a, b| {
        a.name.to_lowercase() == b.name.to_lowercase()
            && a.source_type.to_uppercase() == b.source_type.to_uppercase()
            && a.target_type.to_uppercase() == b.target_type.to_uppercase()
    });

    Ok(RawSchemaProposal {
        entity_types: merged_entity_types,
        relation_types: merged_relation_types,
    })
}

// ===========================================================================
// Two-pass analysis (new)
// ===========================================================================

/// Generate a domain persona from document excerpts.
///
/// Sends a short prompt to the LLM asking it to identify the domain and
/// generate a specialist persona. This is a cheap call (small input/output).
async fn generate_persona(
    documents: &[SampledDocument],
    openai_config: &OpenAiConfig,
) -> anyhow::Result<PersonaResult> {
    let user_prompt = prompt::build_persona_prompt(documents);
    let json_schema = prompt::build_persona_json_schema();

    let system_message =
        "You are a domain identification assistant. Analyze document excerpts and identify the domain. \
         Respond ONLY with valid JSON.".to_string();

    call_openai_generic::<PersonaResult>(
        &system_message,
        &user_prompt,
        json_schema,
        openai_config,
        0.3, // Slightly more creative for persona generation
    )
    .await
}

/// Analyze sampled documents using a two-pass approach.
///
/// 1. **Persona step**: Auto-detect domain if user provided no domain context,
///    otherwise build a simple persona from user input (skip LLM call).
/// 2. **Pass 1 — Entity types**: Discover entity types with persona and few-shot context.
/// 3. **Pass 2 — Relationship types**: Discover relationship types with entity context.
///
/// Returns `(proposal, auto_persona)` where `auto_persona` is the generated persona
/// string if one was auto-generated (None if user provided domain context).
pub async fn analyze_schema_two_pass(
    documents: &[SampledDocument],
    input: &SuggestSchemaInput,
    openai_config: &OpenAiConfig,
) -> anyhow::Result<(RawSchemaProposal, Option<String>)> {
    let total_chars: usize = documents.iter().map(|d| d.content.len()).sum();
    let estimated_tokens = total_chars / 4;

    info!(
        documents = documents.len(),
        estimated_tokens = estimated_tokens,
        model = %openai_config.model,
        "Analyzing schema with two-pass LLM approach"
    );

    // -----------------------------------------------------------------------
    // Step 1: Persona
    // -----------------------------------------------------------------------
    let (persona, auto_persona) = if input.domain_description.is_none() && input.domain_hint.is_none() {
        info!("Generating domain persona from document excerpts");
        let result = generate_persona(documents, openai_config).await?;
        info!(
            domain = %result.domain,
            themes = ?result.key_themes,
            "Auto-detected domain persona"
        );
        let persona_str = result.persona.clone();
        (persona_str.clone(), Some(persona_str))
    } else {
        let domain_ctx = input
            .domain_description
            .as_deref()
            .or(input.domain_hint.as_deref())
            .unwrap_or("general");
        let persona_str = format!(
            "You are a {} domain specialist who analyzes documents to identify \
             key entities and relationships for a knowledge graph.",
            domain_ctx
        );
        info!(domain = domain_ctx, "Using user-provided domain context");
        (persona_str, None)
    };

    // -----------------------------------------------------------------------
    // Step 2: Pass 1 — Entity discovery
    // -----------------------------------------------------------------------
    info!("Pass 1: Discovering entity types");

    let entity_user_prompt = prompt::build_entity_discovery_prompt(documents, &persona, input);
    let entity_json_schema = prompt::build_entity_json_schema();

    let entity_system = format!(
        "{}. Respond ONLY with valid JSON matching the required schema.",
        persona
    );

    let entity_result = call_openai_generic::<RawEntityOnlyProposal>(
        &entity_system,
        &entity_user_prompt,
        entity_json_schema,
        openai_config,
        0.2,
    )
    .await?;

    info!(
        entity_types = entity_result.entity_types.len(),
        "Pass 1 complete: discovered {} entity types",
        entity_result.entity_types.len()
    );

    // -----------------------------------------------------------------------
    // Step 3: Pass 2 — Relationship discovery with entity context
    // -----------------------------------------------------------------------
    info!("Pass 2: Discovering relationship types with entity context");

    let rel_user_prompt = prompt::build_relationship_discovery_prompt(
        documents,
        &persona,
        input,
        &entity_result.entity_types,
    );
    let rel_json_schema = prompt::build_relationship_json_schema();

    let rel_system = format!(
        "{}. Respond ONLY with valid JSON matching the required schema.",
        persona
    );

    let rel_result = call_openai_generic::<RawRelationOnlyProposal>(
        &rel_system,
        &rel_user_prompt,
        rel_json_schema,
        openai_config,
        0.2,
    )
    .await?;

    info!(
        relation_types = rel_result.relation_types.len(),
        "Pass 2 complete: discovered {} relationship types",
        rel_result.relation_types.len()
    );

    // -----------------------------------------------------------------------
    // Step 4: Merge into unified proposal
    // -----------------------------------------------------------------------
    let proposal = RawSchemaProposal {
        entity_types: entity_result.entity_types,
        relation_types: rel_result.relation_types,
    };

    Ok((proposal, auto_persona))
}

// ===========================================================================
// Generic OpenAI call
// ===========================================================================

/// Call OpenAI API with structured output and parse the JSON response.
///
/// Generic over the response type `T` — works for `RawSchemaProposal`,
/// `PersonaResult`, `RawEntityOnlyProposal`, `RawRelationOnlyProposal`, etc.
async fn call_openai_generic<T: DeserializeOwned>(
    system_prompt: &str,
    user_prompt: &str,
    json_schema_def: serde_json::Value,
    openai_config: &OpenAiConfig,
    temperature: f32,
) -> anyhow::Result<T> {
    let mut config = AsyncOpenAIConfig::new().with_api_key(&openai_config.api_key);
    if let Some(ref base_url) = openai_config.base_url {
        config = config.with_api_base(base_url);
    }

    let client = Client::with_config(config);

    let messages: Vec<ChatCompletionRequestMessage> = vec![
        ChatCompletionRequestSystemMessageArgs::default()
            .content(system_prompt)
            .build()?
            .into(),
        ChatCompletionRequestUserMessageArgs::default()
            .content(user_prompt)
            .build()?
            .into(),
    ];

    let schema_name = json_schema_def["name"]
        .as_str()
        .unwrap_or("structured_output")
        .to_string();

    let response_format = ResponseFormat::JsonSchema {
        json_schema: ResponseFormatJsonSchema {
            description: Some(format!("Structured output: {}", schema_name)),
            name: schema_name.clone(),
            schema: Some(json_schema_def["schema"].clone()),
            strict: Some(true),
        },
    };

    let request = CreateChatCompletionRequestArgs::default()
        .model(&openai_config.model)
        .messages(messages)
        .response_format(response_format)
        .temperature(temperature)
        .build()?;

    debug!(schema = %schema_name, "Sending structured request to OpenAI");

    let response = client.chat().create(request).await?;

    let content = response
        .choices
        .first()
        .and_then(|c| c.message.content.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No content in OpenAI response"))?;

    debug!(
        content_len = content.len(),
        schema = %schema_name,
        "Received OpenAI response"
    );

    let parsed: T = serde_json::from_str(content).map_err(|e| {
        warn!(
            error = %e,
            content = &content[..content.len().min(500)],
            schema = %schema_name,
            "Failed to parse LLM response"
        );
        anyhow::anyhow!(
            "Failed to parse LLM {} response: {}. Response prefix: {}",
            schema_name,
            e,
            &content[..content.len().min(200)]
        )
    })?;

    Ok(parsed)
}
