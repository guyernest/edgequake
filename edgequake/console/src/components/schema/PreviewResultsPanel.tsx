import * as DialogPrimitive from '@radix-ui/react-dialog';
import { X, RefreshCw } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { PreviewCountsTable } from './PreviewCountsTable';
import { CoverageHeatmap } from './CoverageHeatmap';

interface PreviewEntityCount {
  typeName: string;
  count: number;
}

interface PreviewRelationCount {
  typeName: string;
  count: number;
}

interface PreviewCoverageRow {
  entityType: string;
  counts: number[];
}

interface PreviewDocumentColumn {
  id: string;
  name: string;
  truncatedName: string;
}

interface PreviewCost {
  inputTokens: number;
  outputTokens: number;
  totalCostUsd: number;
  model: string;
}

export interface PreviewResult {
  status: string;
  entityTypeCounts?: PreviewEntityCount[] | null;
  relationTypeCounts?: PreviewRelationCount[] | null;
  coverageRows?: PreviewCoverageRow[] | null;
  documentColumns?: PreviewDocumentColumn[] | null;
  totalChunks?: number | null;
  totalEntities?: number | null;
  totalRelationships?: number | null;
  cost?: PreviewCost | null;
  processingTimeMs?: number | null;
  documentsCompleted?: number | null;
  documentsTotal?: number | null;
}

interface PreviewResultsPanelProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  result: PreviewResult;
  onRunAgain: () => void;
  isRunning: boolean;
  isStale?: boolean;   // true when schema was edited+saved after this preview ran
  isDirty?: boolean;   // true when schema has unsaved local changes
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

export function PreviewResultsPanel({
  open,
  onOpenChange,
  result,
  onRunAgain,
  isRunning,
  isStale = false,
  isDirty = false,
}: PreviewResultsPanelProps) {
  const entityCounts = result.entityTypeCounts ?? [];
  const relationCounts = result.relationTypeCounts ?? [];
  const coverageRows = result.coverageRows ?? [];
  const documentColumns = result.documentColumns ?? [];

  const totalEntities = result.totalEntities ?? entityCounts.reduce((s, r) => s + r.count, 0);
  const totalRelationships = result.totalRelationships ?? relationCounts.reduce((s, r) => s + r.count, 0);
  const documentsUsed = result.documentsCompleted ?? documentColumns.length;

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange} modal={false}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Content
          className="fixed inset-y-0 right-0 z-50 w-[480px] border-l bg-background p-6 shadow-xl overflow-y-auto transition-transform duration-300 data-[state=open]:translate-x-0 data-[state=closed]:translate-x-full"
          onInteractOutside={(e) => e.preventDefault()}
        >
          {/* Header */}
          <div className="mb-4 flex items-center justify-between">
            <DialogPrimitive.Title className="text-lg font-semibold">
              Preview Results
            </DialogPrimitive.Title>
            <DialogPrimitive.Close asChild>
              <Button variant="ghost" size="sm" className="h-8 w-8 p-0">
                <X className="size-4" />
                <span className="sr-only">Close</span>
              </Button>
            </DialogPrimitive.Close>
          </div>

          {/* Summary bar */}
          <div className="mb-4 rounded-lg border bg-muted/30 p-3">
            <p className="text-sm font-medium">
              {totalEntities} entities, {totalRelationships} relationships from{' '}
              {documentsUsed} documents
              {result.processingTimeMs != null && (
                <> in {formatDuration(result.processingTimeMs)}</>
              )}
            </p>
          </div>

          {/* Stale results banner */}
          {isStale && (
            <div className="mb-4 rounded-lg border border-amber-200 bg-amber-50 p-3 text-sm text-amber-800 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-200">
              <p className="font-medium">Results may be outdated</p>
              <p className="mt-0.5 text-xs">Schema has been modified since this preview. Re-run preview to see updated results.</p>
            </div>
          )}

          {/* Cost line */}
          {result.cost && (
            <p className="mb-4 text-xs text-muted-foreground">
              Cost: ${result.cost.totalCostUsd.toFixed(4)} ({result.cost.model})
            </p>
          )}

          <div className="space-y-6">
            {/* Entity Counts */}
            <PreviewCountsTable
              title="Entity Types"
              counts={entityCounts}
            />

            {/* Relationship Counts */}
            <PreviewCountsTable
              title="Relationship Types"
              counts={relationCounts}
            />

            {/* Coverage Heatmap */}
            {coverageRows.length > 0 && documentColumns.length > 0 && (
              <div>
                <h4 className="mb-2 text-sm font-semibold">Coverage Heatmap</h4>
                <CoverageHeatmap
                  coverageRows={coverageRows}
                  documentColumns={documentColumns}
                />
              </div>
            )}
          </div>

          {/* Footer: Run Again */}
          <div className="mt-6 border-t pt-4">
            <Button
              onClick={onRunAgain}
              disabled={isRunning || isDirty}
              variant="outline"
              className="w-full"
              title={isDirty ? 'Save schema changes before re-running preview' : undefined}
            >
              <RefreshCw className={`mr-2 size-4 ${isRunning ? 'animate-spin' : ''}`} />
              {isRunning ? 'Running...' : 'Run Again'}
            </Button>
            <p className="mt-1.5 text-center text-xs text-muted-foreground">
              {isDirty ? 'Save changes first' : 'Re-samples fresh documents'}
            </p>
          </div>

          <DialogPrimitive.Description className="sr-only">
            Preview extraction results showing entity counts, relationship counts,
            and coverage heatmap per document.
          </DialogPrimitive.Description>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}
