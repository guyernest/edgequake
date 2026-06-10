/**
 * @module SnapshotConfigPanel
 * @description Workspace settings panel for snapshot export destination and mode config.
 *
 * @implements D-03: Free-text URI with inline validation (s3:// or /absolute path)
 * @implements D-04: Mode radio disabled until URI is set; default "write-and-store"
 *
 * Security: T-23-02 — client-side path-traversal guard (reject URIs containing "..");
 * server-side validate_pipeline_config_update is the authoritative defense-in-depth.
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
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group';
import {
  getNamespaceConfig,
  updateNamespaceConfig,
} from '@/lib/api/edgequake';
import { cn } from '@/lib/utils';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Archive, Save } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';

// ============================================================================
// Validation helpers (client-side UX mirrors server-side validate_pipeline_config_update)
// ============================================================================

/**
 * Validates the snapshot URI on blur.
 *
 * Rules (mirroring server-side T-23-02):
 * 1. Empty string is valid ("no snapshot" state — save allowed).
 * 2. Any value containing ".." is rejected (path-traversal guard).
 * 3. Non-empty values must start with "s3://" or "/" (absolute path).
 *
 * Returns null when valid, or a translation key when invalid.
 */
function validateSnapshotUri(uri: string): 'uriErrorBadFormat' | null {
  if (uri === '') return null;
  if (uri.includes('..')) return 'uriErrorBadFormat';
  if (!uri.startsWith('s3://') && !uri.startsWith('/')) return 'uriErrorBadFormat';
  return null;
}

/** Returns true when mode RadioGroup should be enabled (URI non-empty and valid). */
function isModeEnabled(uri: string): boolean {
  if (uri === '') return false;
  return validateSnapshotUri(uri) === null;
}

// ============================================================================
// Component
// ============================================================================

interface SnapshotConfigPanelProps {
  namespace: string;
  /** Called when the user saves a config change — parent wires into needsExtractionRebuild state. */
  onDirty?: () => void;
}

export function SnapshotConfigPanel({ namespace, onDirty }: SnapshotConfigPanelProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  // ---- form state ----
  const [uri, setUri] = useState('');
  const [uriError, setUriError] = useState<string | null>(null);
  const [mode, setMode] = useState('write-and-store');

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
      setUri(config.snapshot_uri ?? '');
      setMode(config.snapshot_mode ?? 'write-and-store');
    }
  }, [config]);

  // ---- mutation ----
  const mutation = useMutation({
    mutationFn: (data: Parameters<typeof updateNamespaceConfig>[1]) =>
      updateNamespaceConfig(namespace, data),
    onSuccess: () => {
      toast.success(t('pipeline.snapshot.saved'));
      queryClient.invalidateQueries({ queryKey: ['namespaceConfig', namespace] });
    },
    onError: (error) => {
      toast.error(t('common.error', 'Failed to save.'), {
        description: error instanceof Error ? error.message : 'Unknown error',
      });
    },
  });

  // ---- handlers ----
  const handleUriBlur = () => {
    // Validate on blur (UI-SPEC: not live while typing)
    const err = validateSnapshotUri(uri);
    setUriError(err ? t(`pipeline.snapshot.${err}`) : null);
  };

  const handleSave = () => {
    // Re-validate on submit
    const err = validateSnapshotUri(uri);
    if (err) {
      setUriError(t(`pipeline.snapshot.${err}`));
      return;
    }

    const update: Parameters<typeof updateNamespaceConfig>[1] = {};
    if (uri === '') {
      update.snapshot_uri_clear = true;
    } else {
      update.snapshot_uri = uri;
      update.snapshot_mode = mode;
    }
    mutation.mutate(update);
    onDirty?.();
  };

  const modeEnabled = isModeEnabled(uri);

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Archive className="h-5 w-5 text-amber-600" />
          {t('pipeline.snapshot.title')}
        </CardTitle>
        <CardDescription>
          {t('pipeline.snapshot.description')}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        {/* URI field (D-03) — free-text, validated on blur */}
        <div className="space-y-1.5">
          <Label htmlFor="snapshot-uri" className="text-sm font-medium">
            {t('pipeline.snapshot.uriLabel')}
          </Label>
          <Input
            id="snapshot-uri"
            type="text"
            placeholder={t('pipeline.snapshot.uriPlaceholder')}
            value={uri}
            onChange={(e) => {
              setUri(e.target.value);
              // Clear error while typing (validation fires on blur)
              if (uriError) setUriError(null);
            }}
            onBlur={handleUriBlur}
            disabled={isLoading || mutation.isPending}
            className={cn(uriError && 'border-destructive')}
          />
          <p className="text-xs text-muted-foreground">
            {t('pipeline.snapshot.uriHint')}
          </p>
          {uriError && (
            <p className="text-xs text-destructive" role="alert">
              {uriError}
            </p>
          )}
          {/* Disabled hint shown when URI is empty — appears below the URI field */}
          {!modeEnabled && uri === '' && (
            <p className="text-xs text-muted-foreground italic">
              {t('pipeline.snapshot.disabledHint')}
            </p>
          )}
        </div>

        {/* Mode radio (D-04) — disabled when URI is empty; always rendered (not hidden) */}
        <div className="space-y-2">
          <Label className="text-sm font-medium">
            {t('pipeline.snapshot.modeLabel')}
          </Label>
          <RadioGroup
            value={mode}
            onValueChange={setMode}
            disabled={!modeEnabled}
            className="space-y-2"
          >
            <div className="flex items-center gap-2">
              <RadioGroupItem
                id="mode-write-and-store"
                value="write-and-store"
                disabled={!modeEnabled}
              />
              <Label
                htmlFor="mode-write-and-store"
                className={cn(
                  'text-sm cursor-pointer',
                  !modeEnabled && 'opacity-50 cursor-not-allowed'
                )}
              >
                {t('pipeline.snapshot.modeWriteAndStore')}
              </Label>
            </div>
            <div className="flex items-center gap-2">
              <RadioGroupItem
                id="mode-snapshot-only"
                value="snapshot-only"
                disabled={!modeEnabled}
              />
              <Label
                htmlFor="mode-snapshot-only"
                className={cn(
                  'text-sm cursor-pointer',
                  !modeEnabled && 'opacity-50 cursor-not-allowed'
                )}
              >
                {t('pipeline.snapshot.modeSnapshotOnly')}
              </Label>
            </div>
          </RadioGroup>
        </div>

        {/* Save button */}
        <div className="flex justify-end">
          <Button
            onClick={handleSave}
            disabled={isLoading || mutation.isPending || !!uriError}
          >
            <Save className="h-4 w-4 mr-2" />
            {t('common.save', 'Save settings')}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
