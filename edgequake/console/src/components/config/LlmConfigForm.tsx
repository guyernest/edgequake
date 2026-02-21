import { useState, useEffect } from 'react';
import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { ProviderModelCascade } from './ProviderModelCascade';
import {
  usePipelineConfig,
  useUpdatePipelineConfig,
} from '@/hooks/usePipelineConfig';
import { LLM_PROVIDERS, EMBEDDING_PROVIDERS } from '@/lib/providers';
import { validatePipelineConfig } from '@/lib/validation';

interface LlmConfigFormProps {
  slug: string;
}

export function LlmConfigForm({ slug }: LlmConfigFormProps) {
  const { data: config, isLoading } = usePipelineConfig(slug);
  const mutation = useUpdatePipelineConfig(slug);

  const [llmProvider, setLlmProvider] = useState('openai');
  const [llmModel, setLlmModel] = useState('gpt-4o');
  const [embeddingProvider, setEmbeddingProvider] = useState('openai');
  const [embeddingModel, setEmbeddingModel] = useState(
    'text-embedding-3-small'
  );
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [initialized, setInitialized] = useState(false);

  // Initialize form from server data
  useEffect(() => {
    if (config && !initialized) {
      setLlmProvider(config.llm_provider ?? 'openai');
      setLlmModel(config.llm_model ?? 'gpt-4o');
      setEmbeddingProvider(config.embedding_provider ?? 'openai');
      setEmbeddingModel(
        config.embedding_model ?? 'text-embedding-3-small'
      );
      setInitialized(true);
    }
  }, [config, initialized]);

  const isPristine =
    initialized &&
    config &&
    llmProvider === (config.llm_provider ?? 'openai') &&
    llmModel === (config.llm_model ?? 'gpt-4o') &&
    embeddingProvider === (config.embedding_provider ?? 'openai') &&
    embeddingModel === (config.embedding_model ?? 'text-embedding-3-small');

  function handleSave() {
    const result = validatePipelineConfig({
      llm_provider: llmProvider,
      llm_model: llmModel,
      embedding_provider: embeddingProvider,
      embedding_model: embeddingModel,
      chunk_size: config?.chunk_size ?? 1000,
      chunk_overlap: config?.chunk_overlap ?? 200,
    });

    if (!result.valid) {
      setErrors(result.errors);
      return;
    }

    setErrors({});

    // Find embedding dimension for the selected model
    const embModel = EMBEDDING_PROVIDERS.flatMap((p) => p.models).find(
      (m) => m.id === embeddingModel
    );

    mutation.mutate({
      llm_provider: llmProvider,
      llm_model: llmModel,
      embedding_provider: embeddingProvider,
      embedding_model: embeddingModel,
      embedding_dimension: embModel?.dimension ?? 1536,
    });
  }

  if (isLoading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-5 w-40" />
        <Skeleton className="h-9 w-full" />
        <Skeleton className="h-9 w-full" />
        <Skeleton className="h-5 w-40" />
        <Skeleton className="h-9 w-full" />
        <Skeleton className="h-9 w-full" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-base font-semibold">LLM Configuration</h3>
        <p className="text-sm text-muted-foreground">
          Select the provider and model for entity extraction. Changes take
          effect on the next pipeline run.
        </p>
      </div>

      <ProviderModelCascade
        providers={LLM_PROVIDERS}
        selectedProvider={llmProvider}
        selectedModel={llmModel}
        onProviderChange={setLlmProvider}
        onModelChange={setLlmModel}
        label="LLM"
        providerError={errors.llm_provider}
        modelError={errors.llm_model}
      />

      <div>
        <h3 className="text-base font-semibold">Embedding Configuration</h3>
        <p className="text-sm text-muted-foreground">
          Select the embedding model for vector storage.
        </p>
      </div>

      <ProviderModelCascade
        providers={EMBEDDING_PROVIDERS}
        selectedProvider={embeddingProvider}
        selectedModel={embeddingModel}
        onProviderChange={setEmbeddingProvider}
        onModelChange={setEmbeddingModel}
        label="Embedding"
        providerError={errors.embedding_provider}
        modelError={errors.embedding_model}
      />

      <Button
        onClick={handleSave}
        disabled={isPristine || mutation.isPending}
      >
        {mutation.isPending ? 'Saving...' : 'Save LLM Settings'}
      </Button>
    </div>
  );
}
