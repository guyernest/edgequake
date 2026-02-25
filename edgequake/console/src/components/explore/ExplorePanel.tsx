import { Skeleton } from '@/components/ui/skeleton';
import { CliCommandDisplay } from '@/components/ingestion/CliCommandDisplay';
import { useNamespaceStatus } from '@/hooks/useNamespaceStatus';

interface ExplorePanelProps {
  slug: string;
  onNavigateToStatus: () => void;
}

/**
 * Returns true only if the pipeline has completed at least one ingestion run,
 * meaning there is data to explore. Uses pipeline status per locked user decision
 * (NOT entity count).
 */
function hasIngestedData(
  status: { status?: string | null } | null | undefined,
): boolean {
  const s = status?.status;
  return s === 'completed' || s === 'completed_with_warnings';
}

export function ExplorePanel({ slug, onNavigateToStatus }: ExplorePanelProps) {
  const { data: status, isLoading } = useNamespaceStatus(slug);

  if (isLoading) {
    return (
      <div className="space-y-3">
        <Skeleton className="h-6 w-48" />
        <Skeleton className="h-4 w-96" />
        <Skeleton className="h-4 w-72" />
      </div>
    );
  }

  if (!hasIngestedData(status)) {
    return (
      <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-12 text-center">
        <h3 className="text-base font-semibold">No data to explore</h3>
        <p className="mt-2 max-w-md text-sm text-muted-foreground">
          This namespace hasn&apos;t been ingested yet. Run the ingestion
          pipeline to extract entities and relationships from your documents
          into the knowledge graph.
        </p>
        <div className="mt-4 w-full max-w-lg">
          <CliCommandDisplay
            command={`edgequake-batch --namespace ${slug} --data ./path run`}
          />
        </div>
        <button
          type="button"
          className="mt-4 cursor-pointer text-sm text-primary underline"
          onClick={onNavigateToStatus}
        >
          Go to Status tab to monitor pipeline progress
        </button>
      </div>
    );
  }

  {/* Data-present placeholder: Phase 11 (entity browser), Phase 12 (query panel), Phase 13 (vector search) */}
  return (
    <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-12 text-center">
      <p className="text-sm text-muted-foreground">
        Explore features will appear here once implemented.
      </p>
    </div>
  );
}
