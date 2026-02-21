import { FileQuestion } from 'lucide-react';
import { PipelinePhaseEntry, type PhaseStatus } from './PipelinePhaseEntry';
import { BatchProgress } from './BatchProgress';
import { DocumentErrorEntry, DocumentErrorSummary } from './DocumentErrorEntry';

/**
 * Pipeline phase names matching the batch pipeline's Phase enum:
 * Prepare, Extract, Embed, Store
 */
const PIPELINE_PHASES = ['Prepare', 'Extract', 'Embed', 'Store'] as const;

/** Maps pipeline status.phase values to phase index */
const PHASE_INDEX: Record<string, number> = {
  preparing: 0,
  extracting: 1,
  embedding: 2,
  storing: 3,
};

export type NamespaceStatusData = {
  status: string;
  phase?: string | null;
  job_id?: string | null;
  total_documents?: number | null;
  processed_documents?: number | null;
  total_chunks?: number | null;
  current_batch?: number | null;
  total_batches?: number | null;
  started_at?: number | null;
  updated_at?: number | null;
  error_summary?: string | null;
};

export type PipelineTimelineProps = {
  status: NamespaceStatusData | null | undefined;
};

function derivePhaseStatuses(
  status: NamespaceStatusData
): { status: PhaseStatus; startedAt?: number | null; completedAt?: number | null }[] {
  const overallStatus = status.status;
  const currentPhase = status.phase;

  // All completed
  if (overallStatus === 'completed') {
    return PIPELINE_PHASES.map(() => ({
      status: 'completed' as PhaseStatus,
      startedAt: status.started_at,
      completedAt: status.updated_at,
    }));
  }

  // Failed state: phases up to current completed, current failed, rest pending
  if (overallStatus === 'failed' || overallStatus === 'error') {
    const failedIdx = currentPhase ? PHASE_INDEX[currentPhase] ?? 0 : 0;
    return PIPELINE_PHASES.map((_, i) => {
      if (i < failedIdx) return { status: 'completed' as PhaseStatus, startedAt: status.started_at };
      if (i === failedIdx) return { status: 'failed' as PhaseStatus, startedAt: status.started_at, completedAt: status.updated_at };
      return { status: 'pending' as PhaseStatus };
    });
  }

  // Active state: determine from current phase
  if (currentPhase && currentPhase in PHASE_INDEX) {
    const activeIdx = PHASE_INDEX[currentPhase]!;
    return PIPELINE_PHASES.map((_, i) => {
      if (i < activeIdx) return { status: 'completed' as PhaseStatus, startedAt: status.started_at };
      if (i === activeIdx) return { status: 'running' as PhaseStatus, startedAt: status.started_at };
      return { status: 'pending' as PhaseStatus };
    });
  }

  // Idle / requested / unknown: all pending
  return PIPELINE_PHASES.map(() => ({ status: 'pending' as PhaseStatus }));
}

export function PipelineTimeline({ status }: PipelineTimelineProps) {
  // Idle / no data state
  if (!status || status.status === 'idle') {
    return (
      <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-12 text-center">
        <FileQuestion className="mb-3 size-10 text-muted-foreground/50" />
        <h3 className="text-base font-semibold">No pipeline runs yet</h3>
        <p className="mt-1 text-sm text-muted-foreground">
          Trigger an ingestion run to see pipeline status here.
        </p>
      </div>
    );
  }

  const phaseData = derivePhaseStatuses(status);
  const isActive = ['running', 'preparing', 'extracting', 'embedding', 'storing'].includes(status.status);
  const isFailed = status.status === 'failed' || status.status === 'error';
  const failedPhaseIdx = status.phase ? PHASE_INDEX[status.phase] ?? 0 : 0;

  // Parse structured errors if available (future enhancement), otherwise use error_summary
  const errors: { documentId?: string; message: string; phase?: string | null }[] = [];
  if (isFailed && status.error_summary) {
    errors.push({
      message: status.error_summary,
      phase: status.phase,
    });
  }

  return (
    <div className="space-y-1">
      {PIPELINE_PHASES.map((phaseName, i) => {
        const data = phaseData[i]!;
        const showErrorsAfter = isFailed && i === failedPhaseIdx && errors.length > 0;

        return (
          <div key={phaseName}>
            <PipelinePhaseEntry
              name={phaseName}
              status={data.status}
              startedAt={data.startedAt}
              completedAt={data.completedAt}
              isLast={i === PIPELINE_PHASES.length - 1 && !showErrorsAfter}
            />

            {/* Error entries after the failed phase */}
            {showErrorsAfter && (
              <div className="ml-0 mt-1">
                {errors.length === 1 && errors[0] ? (
                  <DocumentErrorEntry error={errors[0]} />
                ) : (
                  <>
                    {errors.map((err, errIdx) => (
                      <DocumentErrorEntry key={errIdx} error={err} />
                    ))}
                    {status.processed_documents != null &&
                      status.total_documents != null &&
                      status.total_documents - status.processed_documents > 0 && (
                        <DocumentErrorSummary
                          errorCount={status.total_documents - status.processed_documents}
                          errorSummary={`${status.total_documents - status.processed_documents} documents were not processed`}
                          phase={status.phase}
                        />
                      )}
                  </>
                )}
              </div>
            )}
          </div>
        );
      })}

      {/* Batch progress shown when pipeline is active or has document data */}
      {(isActive || status.total_documents) && (
        <div className="mt-3">
          <BatchProgress
            totalDocuments={status.total_documents}
            processedDocuments={status.processed_documents}
            currentBatch={status.current_batch}
            totalBatches={status.total_batches}
          />
        </div>
      )}
    </div>
  );
}
