import {
  LLM_PROVIDERS,
  EMBEDDING_PROVIDERS,
  getModelsForProvider,
} from './providers';

export interface PipelineConfigInput {
  llm_provider: string;
  llm_model: string;
  embedding_provider: string;
  embedding_model: string;
  chunk_size: number;
  chunk_overlap: number;
  extraction_prompt?: string;
}

export interface ValidationResult {
  valid: boolean;
  errors: Record<string, string>;
}

/**
 * Validate pipeline configuration fields.
 * Returns an object with `valid` boolean and `errors` map keyed by field name.
 */
export function validatePipelineConfig(
  config: PipelineConfigInput
): ValidationResult {
  const errors: Record<string, string> = {};

  // LLM provider
  if (!config.llm_provider) {
    errors.llm_provider = 'LLM provider is required';
  } else if (!LLM_PROVIDERS.some((p) => p.id === config.llm_provider)) {
    errors.llm_provider = 'Invalid LLM provider';
  }

  // LLM model
  if (!config.llm_model) {
    errors.llm_model = 'LLM model is required';
  } else if (config.llm_provider) {
    const models = getModelsForProvider(LLM_PROVIDERS, config.llm_provider);
    if (!models.some((m) => m.id === config.llm_model)) {
      errors.llm_model = 'Invalid model for selected provider';
    }
  }

  // Embedding provider
  if (!config.embedding_provider) {
    errors.embedding_provider = 'Embedding provider is required';
  } else if (
    !EMBEDDING_PROVIDERS.some((p) => p.id === config.embedding_provider)
  ) {
    errors.embedding_provider = 'Invalid embedding provider';
  }

  // Embedding model
  if (!config.embedding_model) {
    errors.embedding_model = 'Embedding model is required';
  } else if (config.embedding_provider) {
    const models = getModelsForProvider(
      EMBEDDING_PROVIDERS,
      config.embedding_provider
    );
    if (!models.some((m) => m.id === config.embedding_model)) {
      errors.embedding_model = 'Invalid model for selected embedding provider';
    }
  }

  // Chunk size
  if (
    !Number.isInteger(config.chunk_size) ||
    config.chunk_size < 100 ||
    config.chunk_size > 10000
  ) {
    errors.chunk_size = 'Chunk size must be an integer between 100 and 10,000';
  }

  // Chunk overlap
  if (
    !Number.isInteger(config.chunk_overlap) ||
    config.chunk_overlap < 0 ||
    config.chunk_overlap > Math.floor(config.chunk_size / 2)
  ) {
    errors.chunk_overlap = `Chunk overlap must be an integer between 0 and ${Math.floor(config.chunk_size / 2)}`;
  }

  // Extraction prompt
  if (
    config.extraction_prompt &&
    config.extraction_prompt.length > 10000
  ) {
    errors.extraction_prompt =
      'Extraction prompt must be 10,000 characters or fewer';
  }

  return {
    valid: Object.keys(errors).length === 0,
    errors,
  };
}
