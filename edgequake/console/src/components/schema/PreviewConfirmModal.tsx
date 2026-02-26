import { Eye, Terminal, AlertTriangle } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Skeleton } from '@/components/ui/skeleton';
import { Progress } from '@/components/ui/progress';

interface PreviewDocumentInfo {
  id: string;
  name: string;
  chunkCount: number;
}

interface PreviewCostEstimateData {
  documentCount: number;
  chunkCount: number;
  estimatedInputTokens: number;
  estimatedOutputTokens: number;
  estimatedCostUsd: number;
  model: string;
  documents: PreviewDocumentInfo[];
}

interface PreviewConfirmModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  costEstimate: PreviewCostEstimateData | null | undefined;
  isLoading: boolean;
  onConfirm: () => void;
  isPreviewRunning: boolean;
  isTriggerPending: boolean;
  documentsCompleted: number;
  documentsTotal: number;
  previewStatus: string | null | undefined;
  cliCommand: string | null | undefined;
  onCancel: () => void;
  costError: Error | null;
  triggerError: Error | null;
}

export function PreviewConfirmModal({
  open,
  onOpenChange,
  costEstimate,
  isLoading,
  onConfirm,
  isPreviewRunning,
  isTriggerPending,
  documentsCompleted,
  documentsTotal,
  previewStatus,
  cliCommand,
  onCancel,
  costError,
  triggerError,
}: PreviewConfirmModalProps) {
  const isTriggered = previewStatus === 'requested' || previewStatus === 'processing';
  const progressPercent =
    documentsTotal > 0
      ? Math.round((documentsCompleted / documentsTotal) * 100)
      : 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Eye className="size-5" />
            Preview Extraction
          </DialogTitle>
          <DialogDescription>
            Run a small extraction preview to validate your schema produces
            meaningful results before triggering a full pipeline run.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {/* Error display */}
          {(costError || triggerError) && (
            <div className="flex items-start gap-2 rounded-lg border border-destructive/50 bg-destructive/10 p-3">
              <AlertTriangle className="mt-0.5 size-4 shrink-0 text-destructive" />
              <p className="text-sm text-destructive">
                {costError?.message ?? triggerError?.message}
              </p>
            </div>
          )}

          {/* Loading state */}
          {isLoading && !costEstimate && (
            <div className="space-y-3">
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-8 w-3/4" />
              <Skeleton className="h-8 w-1/2" />
            </div>
          )}

          {/* Cost estimate (pre-trigger state) */}
          {costEstimate && !isTriggered && (
            <>
              {/* Cost prominently displayed */}
              <div className="rounded-lg border bg-muted/30 p-4 text-center">
                <div className="text-sm text-muted-foreground">
                  Estimated cost
                </div>
                <div className="mt-1 text-3xl font-bold">
                  ${costEstimate.estimatedCostUsd.toFixed(4)}
                </div>
                <div className="mt-1 text-xs text-muted-foreground">
                  Standard API rates (not batch discount)
                </div>
              </div>

              {/* Document list */}
              <div>
                <h4 className="mb-2 text-sm font-medium">
                  Documents to sample
                </h4>
                <div className="rounded-md border">
                  <table className="w-full">
                    <thead>
                      <tr className="border-b bg-muted/50">
                        <th className="p-2 text-left text-xs font-medium text-muted-foreground">
                          Document
                        </th>
                        <th className="w-24 p-2 text-right text-xs font-medium text-muted-foreground">
                          Chunks
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      {costEstimate.documents.map((doc) => (
                        <tr key={doc.id} className="border-b last:border-0">
                          <td className="p-2 text-sm">{doc.name}</td>
                          <td className="p-2 text-right text-sm tabular-nums">
                            {doc.chunkCount}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>

              {/* Summary line */}
              <p className="text-sm text-muted-foreground">
                {costEstimate.documentCount} documents,{' '}
                {costEstimate.chunkCount} chunks, using{' '}
                <span className="font-mono text-xs">
                  {costEstimate.model}
                </span>
              </p>
            </>
          )}

          {/* Post-trigger: CLI command and progress */}
          {isTriggered && (
            <div className="space-y-4">
              {/* CLI command for operator */}
              {cliCommand && (
                <div className="space-y-2">
                  <div className="flex items-center gap-2 text-sm font-medium">
                    <Terminal className="size-4" />
                    Run this command
                  </div>
                  <div className="rounded-md border bg-muted/50 p-3">
                    <code className="break-all text-xs">{cliCommand}</code>
                  </div>
                </div>
              )}

              {/* Status and progress */}
              {previewStatus === 'requested' && (
                <div className="rounded-lg border bg-muted/30 p-4 text-center">
                  <div className="text-sm text-muted-foreground">
                    Waiting for operator to start CLI...
                  </div>
                </div>
              )}

              {previewStatus === 'processing' && (
                <div className="space-y-2">
                  <div className="flex items-center justify-between text-sm">
                    <span>
                      Processing document {documentsCompleted} of{' '}
                      {documentsTotal}...
                    </span>
                    <span className="tabular-nums text-muted-foreground">
                      {progressPercent}%
                    </span>
                  </div>
                  <Progress value={progressPercent} />
                </div>
              )}
            </div>
          )}
        </div>

        <DialogFooter>
          {!isTriggered ? (
            <>
              <Button
                variant="outline"
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button
                onClick={onConfirm}
                disabled={
                  isLoading || isTriggerPending || !costEstimate
                }
              >
                {isTriggerPending ? 'Starting...' : 'Run Preview'}
              </Button>
            </>
          ) : (
            <Button
              variant="outline"
              onClick={onCancel}
            >
              {previewStatus === 'processing'
                ? 'Cancel (keep partial results)'
                : 'Close'}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
