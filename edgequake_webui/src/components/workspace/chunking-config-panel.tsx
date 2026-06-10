/**
 * @module ChunkingConfigPanel
 * @description Workspace settings panel for hierarchical header-aware chunking config.
 *
 * @implements D-06: Master toggle OFF by default
 * @implements D-07: Phase 22 defaults pre-filled on toggle ON
 * @implements D-08: range + ratio validation (target 64–2048, overlap < target AND <= 50%)
 *
 * Saves through updateNamespaceConfig → PUT /namespaces/{ns}/config
 */
'use client';

import { Button } from '@/components/ui/button';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Switch } from '@/components/ui/switch';
import {
  getNamespaceConfig,
  updateNamespaceConfig,
} from '@/lib/api/edgequake';
import { cn } from '@/lib/utils';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Layers, Save } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';

// ============================================================================
// Validation helpers (mirroring server-side validate_pipeline_config_update)
// ============================================================================

/** Returns true when target_tokens is within the 64–2048 range. */
function targetInRange(v: number): boolean {
  return v >= 64 && v <= 2048;
}

/**
 * Returns true when overlap satisfies all rules:
 * - overlap >= 0
 * - overlap < target  (strictly less than)
 * - overlap / target <= 0.5  (at most 50%)
 */
function isOverlapValid(target: number, overlap: number): boolean {
  if (overlap < 0) return false;
  if (overlap >= target) return false;
  if (target > 0 && overlap / target > 0.5) return false;
  return true;
}

/**
 * Returns the overlap-to-target ratio as a percentage string with one decimal.
 * e.g. 38 / 256 → "14.8"
 */
function overlapPercent(target: number, overlap: number): string {
  if (target <= 0) return '0.0';
  return ((overlap / target) * 100).toFixed(1);
}

// ============================================================================
// Phase 22 defaults (D-07) — applied when toggle turns ON from OFF
// ============================================================================

const PHASE22_DEFAULTS = {
  chunking_strategy: 'heading_boundary',
  target_tokens: 256,
  overlap_tokens: 38,
  prepend_header_path: true,
} as const;

// ============================================================================
// Component
// ============================================================================

interface ChunkingConfigPanelProps {
  namespace: string;
  /** Called when the user saves a config change — parent wires into needsExtractionRebuild state. */
  onDirty?: () => void;
}

export function ChunkingConfigPanel({ namespace, onDirty }: ChunkingConfigPanelProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  // ---- form state ----
  const [chunkingEnabled, setChunkingEnabled] = useState(false);
  const [strategy, setStrategy] = useState<string>(PHASE22_DEFAULTS.chunking_strategy);
  const [targetTokens, setTargetTokens] = useState<number>(PHASE22_DEFAULTS.target_tokens);
  const [overlapTokens, setOverlapTokens] = useState<number>(PHASE22_DEFAULTS.overlap_tokens);
  const [prependHeader, setPrependHeader] = useState<boolean>(PHASE22_DEFAULTS.prepend_header_path);

  // ---- remote state ----
  const { data: config, isLoading } = useQuery({
    queryKey: ['namespaceConfig', namespace],
    queryFn: () => getNamespaceConfig(namespace),
    enabled: !!namespace,
    staleTime: 30000,
  });

  // Sync form from server config on first load
  useEffect(() => {
    if (config) {
      setChunkingEnabled(config.chunking_enabled);
      setStrategy(config.chunking_strategy ?? PHASE22_DEFAULTS.chunking_strategy);
      setTargetTokens(config.target_tokens);
      setOverlapTokens(config.overlap_tokens);
      setPrependHeader(config.prepend_header_path);
    }
  }, [config]);

  // ---- mutation ----
  const mutation = useMutation({
    mutationFn: (data: Parameters<typeof updateNamespaceConfig>[1]) =>
      updateNamespaceConfig(namespace, data),
    onSuccess: () => {
      toast.success(t('pipeline.chunking.saved'));
      queryClient.invalidateQueries({ queryKey: ['namespaceConfig', namespace] });
    },
    onError: (error) => {
      toast.error(t('common.error', 'Failed to save.'), {
        description: error instanceof Error ? error.message : 'Unknown error',
      });
    },
  });

  // ---- validation ----
  const targetError = chunkingEnabled && !targetInRange(targetTokens);
  const overlapError = chunkingEnabled && !isOverlapValid(targetTokens, overlapTokens);
  const hasValidationError = targetError || overlapError;
  const pct = overlapPercent(targetTokens, overlapTokens);

  // ---- handlers ----
  const handleToggle = (enabled: boolean) => {
    setChunkingEnabled(enabled);
    if (enabled && !config?.chunking_enabled) {
      // Pre-fill Phase 22 defaults when turning ON from OFF (D-07)
      setStrategy(PHASE22_DEFAULTS.chunking_strategy);
      setTargetTokens(PHASE22_DEFAULTS.target_tokens);
      setOverlapTokens(PHASE22_DEFAULTS.overlap_tokens);
      setPrependHeader(PHASE22_DEFAULTS.prepend_header_path);
    }
  };

  const handleSave = () => {
    if (hasValidationError) return;

    if (!chunkingEnabled) {
      mutation.mutate({ chunking_enabled: false });
    } else {
      mutation.mutate({
        chunking_enabled: true,
        chunking_strategy: strategy,
        target_tokens: targetTokens,
        overlap_tokens: overlapTokens,
        prepend_header_path: prependHeader,
      });
    }
    onDirty?.();
  };

  // ---- sub-fields disabled class (D-06: visible-but-disabled, NOT hidden) ----
  const subFieldsClass = chunkingEnabled ? '' : 'opacity-50 pointer-events-none';

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Layers className="h-5 w-5 text-indigo-600" />
          {t('pipeline.chunking.title')}
        </CardTitle>
        <CardDescription>
          {t('pipeline.chunking.description')}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        {/* Master toggle (D-06) */}
        <div className="flex items-start gap-3">
          <Switch
            id="chunking-enabled"
            checked={chunkingEnabled}
            onCheckedChange={handleToggle}
            disabled={isLoading || mutation.isPending}
          />
          <div className="space-y-0.5">
            <Label htmlFor="chunking-enabled" className="text-sm font-medium cursor-pointer">
              {t('pipeline.chunking.enableLabel')}
            </Label>
            <p className="text-xs text-muted-foreground">
              {t('pipeline.chunking.enableDescription')}
            </p>
          </div>
        </div>

        {/* Sub-fields — always in DOM, opacity-50 pointer-events-none when OFF (D-06 visible-but-disabled) */}
        <div className={cn('space-y-4', subFieldsClass)}>
          {/* Chunking strategy (D-07: bound to chunking_strategy field — NOT bare strategy) */}
          <div className="space-y-1.5">
            <Label htmlFor="chunking-strategy" className="text-sm font-medium">
              {t('pipeline.chunking.strategyLabel')}
            </Label>
            <Select
              value={strategy}
              onValueChange={setStrategy}
              disabled={!chunkingEnabled}
            >
              <SelectTrigger id="chunking-strategy" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="token">
                  {t('pipeline.chunking.strategyToken')}
                </SelectItem>
                <SelectItem value="heading_boundary">
                  {t('pipeline.chunking.strategyHeadingBoundary')}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>

          {/* Target tokens (D-07 / D-08: 64–2048) */}
          <div className="space-y-1.5">
            <Label htmlFor="target-tokens" className="text-sm font-medium">
              {t('pipeline.chunking.targetTokensLabel')}
            </Label>
            <Input
              id="target-tokens"
              type="number"
              min={64}
              max={2048}
              value={targetTokens}
              onChange={(e) => setTargetTokens(Number(e.target.value))}
              disabled={!chunkingEnabled}
              className={cn(targetError && 'border-destructive')}
            />
            <p className="text-xs text-muted-foreground">
              {t('pipeline.chunking.targetTokensHelper')}
            </p>
            {targetError && (
              <p className="text-xs text-destructive" role="alert">
                {t('pipeline.chunking.targetTokensError')}
              </p>
            )}
          </div>

          {/* Overlap tokens (D-08: < target AND <= 50%) */}
          <div className="space-y-1.5">
            <Label htmlFor="overlap-tokens" className="text-sm font-medium">
              {t('pipeline.chunking.overlapLabel')}
            </Label>
            <Input
              id="overlap-tokens"
              type="number"
              min={0}
              value={overlapTokens}
              onChange={(e) => setOverlapTokens(Number(e.target.value))}
              disabled={!chunkingEnabled}
              className={cn(overlapError && 'border-destructive')}
            />
            <p className="text-xs text-muted-foreground">
              {t('pipeline.chunking.overlapHint', {
                n: overlapTokens,
                pct,
              })}
            </p>
            {overlapError && (
              <p className="text-xs text-destructive" role="alert">
                {t('pipeline.chunking.overlapError')}
              </p>
            )}
          </div>

          {/* Prepend header path / breadcrumb toggle (D-07) */}
          <div className="flex items-start gap-3">
            <Switch
              id="prepend-header"
              checked={prependHeader}
              onCheckedChange={setPrependHeader}
              disabled={!chunkingEnabled}
            />
            <div className="space-y-0.5">
              <Label htmlFor="prepend-header" className="text-sm font-medium cursor-pointer">
                {t('pipeline.chunking.breadcrumbLabel')}
              </Label>
              <p className="text-xs text-muted-foreground">
                {t('pipeline.chunking.breadcrumbDescription')}
              </p>
            </div>
          </div>
        </div>

        {/* Save button — disabled while validation error present */}
        <div className="flex justify-end">
          <Button
            onClick={handleSave}
            disabled={isLoading || mutation.isPending || hasValidationError}
          >
            <Save className="h-4 w-4 mr-2" />
            {t('common.save', 'Save settings')}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
