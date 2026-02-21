import { Label } from '@/components/ui/label';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import type { ProviderDefinition } from '@/lib/providers';
import { getModelsForProvider } from '@/lib/providers';

interface ProviderModelCascadeProps {
  providers: ProviderDefinition[];
  selectedProvider: string;
  selectedModel: string;
  onProviderChange: (providerId: string) => void;
  onModelChange: (modelId: string) => void;
  label: string;
  providerError?: string;
  modelError?: string;
}

export function ProviderModelCascade({
  providers,
  selectedProvider,
  selectedModel,
  onProviderChange,
  onModelChange,
  label,
  providerError,
  modelError,
}: ProviderModelCascadeProps) {
  const models = getModelsForProvider(providers, selectedProvider);

  function handleProviderChange(providerId: string) {
    onProviderChange(providerId);
    // Reset model to first available when provider changes
    const providerModels = getModelsForProvider(providers, providerId);
    if (providerModels.length > 0) {
      onModelChange(providerModels[0].id);
    }
  }

  return (
    <div className="space-y-3">
      <div className="space-y-1.5">
        <Label>{label} Provider</Label>
        <Select value={selectedProvider} onValueChange={handleProviderChange}>
          <SelectTrigger aria-invalid={!!providerError}>
            <SelectValue placeholder="Select provider" />
          </SelectTrigger>
          <SelectContent>
            {providers.map((provider) => (
              <SelectItem key={provider.id} value={provider.id}>
                {provider.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {providerError && (
          <p className="text-xs text-destructive">{providerError}</p>
        )}
      </div>

      <div className="space-y-1.5">
        <Label>{label} Model</Label>
        <Select
          value={selectedModel}
          onValueChange={onModelChange}
          disabled={models.length === 0}
        >
          <SelectTrigger aria-invalid={!!modelError}>
            <SelectValue placeholder="Select model" />
          </SelectTrigger>
          <SelectContent>
            {models.map((model) => (
              <SelectItem key={model.id} value={model.id}>
                {model.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {modelError && (
          <p className="text-xs text-destructive">{modelError}</p>
        )}
      </div>
    </div>
  );
}
