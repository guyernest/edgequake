import { useState, useEffect } from 'react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
import { Skeleton } from '@/components/ui/skeleton';
import {
  usePipelineConfig,
  useUpdatePipelineConfig,
} from '@/hooks/usePipelineConfig';

interface ExtractionConfigFormProps {
  slug: string;
}

export function ExtractionConfigForm({ slug }: ExtractionConfigFormProps) {
  const { data: config, isLoading } = usePipelineConfig(slug);
  const mutation = useUpdatePipelineConfig(slug);

  const [chunkSize, setChunkSize] = useState(1000);
  const [chunkOverlap, setChunkOverlap] = useState(200);
  const [extractionPrompt, setExtractionPrompt] = useState('');
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [initialized, setInitialized] = useState(false);

  // Initialize form from server data
  useEffect(() => {
    if (config && !initialized) {
      setChunkSize(config.chunk_size ?? 1000);
      setChunkOverlap(config.chunk_overlap ?? 200);
      setExtractionPrompt(config.extraction_prompt ?? '');
      setInitialized(true);
    }
  }, [config, initialized]);

  const isPristine =
    initialized &&
    config &&
    chunkSize === (config.chunk_size ?? 1000) &&
    chunkOverlap === (config.chunk_overlap ?? 200) &&
    extractionPrompt === (config.extraction_prompt ?? '');

  function validate(): boolean {
    const newErrors: Record<string, string> = {};

    if (!Number.isInteger(chunkSize) || chunkSize < 100 || chunkSize > 10000) {
      newErrors.chunk_size =
        'Chunk size must be an integer between 100 and 10,000';
    }

    const maxOverlap = Math.floor(chunkSize / 2);
    if (
      !Number.isInteger(chunkOverlap) ||
      chunkOverlap < 0 ||
      chunkOverlap > maxOverlap
    ) {
      newErrors.chunk_overlap = `Chunk overlap must be an integer between 0 and ${maxOverlap}`;
    }

    if (extractionPrompt.length > 10000) {
      newErrors.extraction_prompt =
        'Extraction prompt must be 10,000 characters or fewer';
    }

    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  }

  function handleSave() {
    if (!validate()) return;

    mutation.mutate({
      chunk_size: chunkSize,
      chunk_overlap: chunkOverlap,
      extraction_prompt: extractionPrompt || undefined,
    });
  }

  if (isLoading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-5 w-40" />
        <Skeleton className="h-9 w-full" />
        <Skeleton className="h-9 w-full" />
        <Skeleton className="h-20 w-full" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-base font-semibold">Extraction Parameters</h3>
        <p className="text-sm text-muted-foreground">
          Configure chunking and extraction settings. Changes take effect on the
          next pipeline run.
        </p>
      </div>

      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-1.5">
          <Label htmlFor="chunk-size">Chunk Size (tokens)</Label>
          <Input
            id="chunk-size"
            type="number"
            min={100}
            max={10000}
            value={chunkSize}
            onChange={(e) => setChunkSize(Number(e.target.value))}
            aria-invalid={!!errors.chunk_size}
          />
          {errors.chunk_size && (
            <p className="text-xs text-destructive">{errors.chunk_size}</p>
          )}
        </div>

        <div className="space-y-1.5">
          <Label htmlFor="chunk-overlap">Chunk Overlap (tokens)</Label>
          <Input
            id="chunk-overlap"
            type="number"
            min={0}
            max={Math.floor(chunkSize / 2)}
            value={chunkOverlap}
            onChange={(e) => setChunkOverlap(Number(e.target.value))}
            aria-invalid={!!errors.chunk_overlap}
          />
          {errors.chunk_overlap && (
            <p className="text-xs text-destructive">{errors.chunk_overlap}</p>
          )}
        </div>
      </div>

      <div className="space-y-1.5">
        <Label htmlFor="extraction-prompt">
          Extraction Prompt{' '}
          <span className="font-normal text-muted-foreground">(optional)</span>
        </Label>
        <Textarea
          id="extraction-prompt"
          placeholder="Custom extraction prompt for entity and relationship extraction..."
          rows={4}
          value={extractionPrompt}
          onChange={(e) => setExtractionPrompt(e.target.value)}
          aria-invalid={!!errors.extraction_prompt}
        />
        <div className="flex justify-between">
          {errors.extraction_prompt ? (
            <p className="text-xs text-destructive">
              {errors.extraction_prompt}
            </p>
          ) : (
            <span />
          )}
          <p className="text-xs text-muted-foreground">
            {extractionPrompt.length.toLocaleString()} / 10,000
          </p>
        </div>
      </div>

      <Button
        onClick={handleSave}
        disabled={isPristine || mutation.isPending}
      >
        {mutation.isPending ? 'Saving...' : 'Save Extraction Settings'}
      </Button>
    </div>
  );
}
