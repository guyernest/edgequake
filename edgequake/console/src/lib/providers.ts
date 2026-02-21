export interface ModelDefinition {
  id: string;
  name: string;
  dimension?: number;
}

export interface ProviderDefinition {
  id: string;
  name: string;
  models: ModelDefinition[];
}

export const LLM_PROVIDERS: ProviderDefinition[] = [
  {
    id: 'openai',
    name: 'OpenAI',
    models: [
      { id: 'gpt-4o', name: 'GPT-4o' },
      { id: 'gpt-4o-mini', name: 'GPT-4o Mini' },
      { id: 'gpt-4-turbo', name: 'GPT-4 Turbo' },
      { id: 'gpt-3.5-turbo', name: 'GPT-3.5 Turbo' },
    ],
  },
  {
    id: 'anthropic',
    name: 'Anthropic',
    models: [
      { id: 'claude-sonnet-4-20250514', name: 'Claude Sonnet 4' },
      { id: 'claude-3-5-haiku-20241022', name: 'Claude 3.5 Haiku' },
    ],
  },
];

export const EMBEDDING_PROVIDERS: ProviderDefinition[] = [
  {
    id: 'openai',
    name: 'OpenAI',
    models: [
      {
        id: 'text-embedding-3-small',
        name: 'text-embedding-3-small',
        dimension: 1536,
      },
      {
        id: 'text-embedding-3-large',
        name: 'text-embedding-3-large',
        dimension: 3072,
      },
      {
        id: 'text-embedding-ada-002',
        name: 'text-embedding-ada-002',
        dimension: 1536,
      },
    ],
  },
];

/**
 * Get models for a given provider from a provider list.
 * Returns empty array if provider not found.
 */
export function getModelsForProvider(
  providers: ProviderDefinition[],
  providerId: string
): ModelDefinition[] {
  return providers.find((p) => p.id === providerId)?.models ?? [];
}
