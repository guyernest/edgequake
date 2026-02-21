import { useEffect, useState } from 'react';
import { format } from 'date-fns';
import { Check, X, Minus, Loader2 } from 'lucide-react';
import { cn } from '@/lib/utils';

export type PhaseStatus = 'pending' | 'running' | 'completed' | 'failed' | 'skipped';

export type PipelinePhaseEntryProps = {
  name: string;
  status: PhaseStatus;
  startedAt?: number | null;
  completedAt?: number | null;
  isLast?: boolean;
};

function formatTimestamp(epochMs: number): string {
  return format(new Date(epochMs), 'h:mm a');
}

function formatDuration(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes === 0) return `${seconds}s`;
  return `${minutes}m ${seconds}s`;
}

function ElapsedTimer({ startedAt }: { startedAt: number }) {
  const [elapsed, setElapsed] = useState(() => Date.now() - startedAt);

  useEffect(() => {
    const interval = setInterval(() => {
      setElapsed(Date.now() - startedAt);
    }, 1000);
    return () => clearInterval(interval);
  }, [startedAt]);

  return <span className="text-blue-600 dark:text-blue-400">(running for {formatDuration(elapsed)})</span>;
}

function StatusIcon({ status }: { status: PhaseStatus }) {
  switch (status) {
    case 'completed':
      return (
        <div className="flex size-6 items-center justify-center rounded-full bg-green-100 dark:bg-green-900/40">
          <Check className="size-3.5 text-green-600 dark:text-green-400" />
        </div>
      );
    case 'running':
      return (
        <div className="flex size-6 items-center justify-center rounded-full bg-blue-100 dark:bg-blue-900/40">
          <Loader2 className="size-3.5 animate-spin text-blue-600 dark:text-blue-400" />
        </div>
      );
    case 'failed':
      return (
        <div className="flex size-6 items-center justify-center rounded-full bg-red-100 dark:bg-red-900/40">
          <X className="size-3.5 text-red-600 dark:text-red-400" />
        </div>
      );
    case 'skipped':
      return (
        <div className="flex size-6 items-center justify-center rounded-full bg-muted">
          <Minus className="size-3.5 text-muted-foreground" />
        </div>
      );
    case 'pending':
    default:
      return (
        <div className="size-6 rounded-full border-2 border-muted-foreground/30 bg-background" />
      );
  }
}

export function PipelinePhaseEntry({
  name,
  status,
  startedAt,
  completedAt,
  isLast = false,
}: PipelinePhaseEntryProps) {
  const statusLabels: Record<PhaseStatus, string> = {
    pending: 'Pending',
    running: 'In Progress',
    completed: 'Completed',
    failed: 'Failed',
    skipped: 'Skipped',
  };

  const duration =
    startedAt && completedAt ? completedAt - startedAt : null;

  return (
    <div className="relative flex gap-3">
      {/* Vertical connector line */}
      {!isLast && (
        <div className="absolute left-3 top-8 -bottom-2 w-px bg-border" />
      )}

      {/* Status icon */}
      <div className="relative z-10 mt-0.5 shrink-0">
        <StatusIcon status={status} />
      </div>

      {/* Phase content */}
      <div className={cn("min-w-0 flex-1 pb-6", isLast && "pb-0")}>
        <div className="flex items-baseline gap-2">
          <span className="font-semibold text-sm">{name}</span>
          <span
            className={cn(
              "text-xs",
              status === 'completed' && 'text-green-600 dark:text-green-400',
              status === 'running' && 'text-blue-600 dark:text-blue-400',
              status === 'failed' && 'text-red-600 dark:text-red-400',
              status === 'pending' && 'text-muted-foreground',
              status === 'skipped' && 'text-muted-foreground'
            )}
          >
            {statusLabels[status]}
          </span>
        </div>

        {/* Timestamps */}
        {startedAt && (
          <div className="mt-1 text-xs text-muted-foreground">
            {status === 'running' ? (
              <span>
                Started: {formatTimestamp(startedAt)}{' '}
                <ElapsedTimer startedAt={startedAt} />
              </span>
            ) : completedAt ? (
              <span>
                {formatTimestamp(startedAt)} &mdash; {formatTimestamp(completedAt)}
                {duration != null && (
                  <span className="ml-1.5 text-foreground/70">({formatDuration(duration)})</span>
                )}
              </span>
            ) : (
              <span>Started: {formatTimestamp(startedAt)}</span>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
