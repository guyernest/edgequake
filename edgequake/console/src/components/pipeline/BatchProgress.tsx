import { Progress } from '@/components/ui/progress';

export type BatchProgressProps = {
  totalDocuments: number | null | undefined;
  processedDocuments: number | null | undefined;
  currentBatch: number | null | undefined;
  totalBatches: number | null | undefined;
  batchSize?: number | null;
  offset?: number | null;
};

export function BatchProgress({
  totalDocuments,
  processedDocuments,
  currentBatch,
  totalBatches,
  batchSize,
  offset,
}: BatchProgressProps) {
  if (!totalDocuments) return null;

  const processed = processedDocuments ?? 0;
  const percentage = totalDocuments > 0
    ? Math.min(Math.round((processed / totalDocuments) * 100), 100)
    : 0;

  // Calculate batch range for display
  const rangeStart = offset != null ? offset + 1 : null;
  const rangeEnd = rangeStart != null && batchSize != null
    ? Math.min(rangeStart + batchSize - 1, totalDocuments)
    : null;

  return (
    <div className="ml-9 rounded-md border bg-muted/30 p-3">
      <div className="mb-2 flex items-center justify-between text-xs">
        <span className="font-medium">Document Progress</span>
        <span className="text-muted-foreground">
          {processed.toLocaleString()} / {totalDocuments.toLocaleString()} ({percentage}%)
        </span>
      </div>

      <Progress value={percentage} className="h-1.5" />

      <div className="mt-2 flex items-center justify-between text-xs text-muted-foreground">
        {rangeStart != null && rangeEnd != null ? (
          <span>
            Processing documents {rangeStart.toLocaleString()}&ndash;{rangeEnd.toLocaleString()} of {totalDocuments.toLocaleString()}
          </span>
        ) : (
          <span>
            {processed.toLocaleString()} of {totalDocuments.toLocaleString()} documents processed
          </span>
        )}

        {currentBatch != null && totalBatches != null && (
          <span>
            Batch {currentBatch} of {totalBatches}
          </span>
        )}
      </div>
    </div>
  );
}
