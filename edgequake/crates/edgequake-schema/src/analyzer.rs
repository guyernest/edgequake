//! LLM-based schema analysis using OpenAI.
//!
//! Sends sampled documents to OpenAI and parses the structured JSON response
//! into raw entity and relation type proposals.

use async_openai::{
    config::OpenAIConfig as AsyncOpenAIConfig,
    types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs,
        ResponseFormat, ResponseFormatJsonSchema,
    },
    Client,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::prompt::{build_schema_prompt, build_json_schema};
use crate::sampler::SampledDocument;

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

/// Maximum tokens to estimate before considering batching.
/// GPT-4.1-mini has 1M context; we leave buffer for prompt template + response.
const MAX_ESTIMATED_TOKENS: usize = 900_000;

/// Analyze sampled documents using OpenAI to propose a schema.
///
/// Constructs the prompt, sends it to the LLM, and parses the JSON response.
/// If the total text exceeds the context window, splits into batches and merges.
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
        "Analyzing schema with LLM"
    );

    if estimated_tokens <= MAX_ESTIMATED_TOKENS {
        // Single prompt: all documents fit
        analyze_single_batch(documents, domain_hint, openai_config).await
    } else {
        // Multi-batch: split documents and merge results
        info!(
            estimated_tokens = estimated_tokens,
            max_tokens = MAX_ESTIMATED_TOKENS,
            "Text exceeds context window, splitting into batches"
        );
        analyze_multi_batch(documents, domain_hint, openai_config).await
    }
}

/// Analyze a single batch of documents.
async fn analyze_single_batch(
    documents: &[SampledDocument],
    domain_hint: Option<&str>,
    openai_config: &OpenAiConfig,
) -> anyhow::Result<RawSchemaProposal> {
    let prompt = build_schema_prompt(documents, domain_hint);
    let json_schema = build_json_schema();

    let system_message = format!(
        "You are a schema analyst. Respond ONLY with valid JSON matching this schema:\n{}",
        serde_json::to_string_pretty(&json_schema["schema"])?
    );

    call_openai(&system_message, &prompt, openai_config).await
}

/// Analyze documents in multiple batches, then merge results.
async fn analyze_multi_batch(
    documents: &[SampledDocument],
    domain_hint: Option<&str>,
    openai_config: &OpenAiConfig,
) -> anyhow::Result<RawSchemaProposal> {
    // Split into batches that fit within context
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

        let batch_docs: Vec<SampledDocument> =
            batch.iter().map(|d| (*d).clone()).collect();
        let result = analyze_single_batch(&batch_docs, domain_hint, openai_config).await?;

        merged_entity_types.extend(result.entity_types);
        merged_relation_types.extend(result.relation_types);
    }

    // Deduplicate by name (keep highest frequency)
    merged_entity_types.sort_by(|a, b| b.frequency.cmp(&a.frequency));
    merged_entity_types.dedup_by(|a, b| {
        if a.name.to_uppercase() == b.name.to_uppercase() {
            // Keep b (which has higher or equal frequency due to sort)
            true
        } else {
            false
        }
    });

    merged_relation_types.sort_by(|a, b| b.frequency.cmp(&a.frequency));
    merged_relation_types.dedup_by(|a, b| {
        if a.name.to_lowercase() == b.name.to_lowercase()
            && a.source_type.to_uppercase() == b.source_type.to_uppercase()
            && a.target_type.to_uppercase() == b.target_type.to_uppercase()
        {
            true
        } else {
            false
        }
    });

    Ok(RawSchemaProposal {
        entity_types: merged_entity_types,
        relation_types: merged_relation_types,
    })
}

/// Call OpenAI API and parse the JSON response.
async fn call_openai(
    system_prompt: &str,
    user_prompt: &str,
    openai_config: &OpenAiConfig,
) -> anyhow::Result<RawSchemaProposal> {
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

    // Use Structured Outputs (json_schema response format) for reliable JSON
    let json_schema_def = crate::prompt::build_json_schema();
    let response_format = ResponseFormat::JsonSchema {
        json_schema: ResponseFormatJsonSchema {
            description: Some("Schema proposal with entity and relation types".to_string()),
            name: "schema_proposal".to_string(),
            schema: Some(json_schema_def["schema"].clone()),
            strict: Some(true),
        },
    };

    let request = CreateChatCompletionRequestArgs::default()
        .model(&openai_config.model)
        .messages(messages)
        .response_format(response_format)
        .temperature(0.2_f32)
        .build()?;

    debug!("Sending schema analysis request to OpenAI");

    let response = client.chat().create(request).await?;

    let content = response
        .choices
        .first()
        .and_then(|c| c.message.content.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No content in OpenAI response"))?;

    debug!(content_len = content.len(), "Received schema analysis response");

    // Parse JSON response
    let proposal: RawSchemaProposal = serde_json::from_str(content).map_err(|e| {
        warn!(
            error = %e,
            content = &content[..content.len().min(500)],
            "Failed to parse LLM response as RawSchemaProposal"
        );
        anyhow::anyhow!(
            "Failed to parse LLM schema response: {}. Response prefix: {}",
            e,
            &content[..content.len().min(200)]
        )
    })?;

    info!(
        entity_types = proposal.entity_types.len(),
        relation_types = proposal.relation_types.len(),
        "Schema analysis complete"
    );

    Ok(proposal)
}
