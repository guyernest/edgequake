import { ChevronRight, AlertCircle } from 'lucide-react';
import { format } from 'date-fns';
import {
  Collapsible,
  CollapsibleTrigger,
  CollapsibleContent,
} from '@/components/ui/collapsible';
import { cn } from '@/lib/utils';

export type DocumentError = {
  documentId?: string;
  message: string;
  stackTrace?: string | null;
  phase?: string | null;
  timestamp?: number | null;
};

export type DocumentErrorEntryProps = {
  error: DocumentError;
};

export function DocumentErrorEntry({ error }: DocumentErrorEntryProps) {
  const briefMessage = error.message.split('\n')[0] ?? error.message;
  const hasDetails = (error.stackTrace != null && error.stackTrace.length > 0) ||
    error.message.includes('\n') ||
    error.phase != null;

  return (
    <Collapsible>
      <div className="relative flex gap-3">
        {/* Red dot on timeline */}
        <div className="relative z-10 mt-0.5 shrink-0">
          <div className="flex size-6 items-center justify-center rounded-full bg-red-100 dark:bg-red-900/40">
            <AlertCircle className="size-3.5 text-red-600 dark:text-red-400" />
          </div>
        </div>

        {/* Error content */}
        <div className="min-w-0 flex-1 pb-4">
          <div
            className={cn(
              'rounded-md border border-red-200 bg-red-50 dark:border-red-800/50 dark:bg-red-950/20',
              'border-l-4 border-l-red-400 dark:border-l-red-600'
            )}
          >
            <CollapsibleTrigger
              className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm"
              disabled={!hasDetails}
            >
              <ChevronRight
                className={cn(
                  'size-3.5 shrink-0 text-red-400 transition-transform duration-200',
                  'data-[state=open]:rotate-90',
                  !hasDetails && 'invisible'
                )}
              />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  {error.documentId && (
                    <span className="shrink-0 rounded bg-red-100 px-1.5 py-0.5 font-mono text-xs text-red-700 dark:bg-red-900/40 dark:text-red-300">
                      {error.documentId}
                    </span>
                  )}
                  <span className="truncate text-sm text-red-800 dark:text-red-200">
                    {briefMessage}
                  </span>
                </div>
              </div>
            </CollapsibleTrigger>

            {hasDetails && (
              <CollapsibleContent>
                <div className="border-t border-red-200 px-3 py-2 dark:border-red-800/50">
                  {/* Full error message (if multiline) */}
                  {error.message.includes('\n') && (
                    <div className="mb-2">
                      <div className="mb-1 text-xs font-medium text-red-700 dark:text-red-300">
                        Error Message
                      </div>
                      <p className="text-xs text-red-600 whitespace-pre-wrap dark:text-red-400">
                        {error.message}
                      </p>
                    </div>
                  )}

                  {/* Stack trace */}
                  {error.stackTrace && (
                    <div className="mb-2">
                      <div className="mb-1 text-xs font-medium text-red-700 dark:text-red-300">
                        Stack Trace
                      </div>
                      <pre className="overflow-x-auto rounded bg-red-100/50 p-2 font-mono text-xs text-red-700 dark:bg-red-950/40 dark:text-red-300">
                        {error.stackTrace}
                      </pre>
                    </div>
                  )}

                  {/* Metadata */}
                  <div className="flex gap-4 text-xs text-red-500 dark:text-red-400">
                    {error.phase && (
                      <span>
                        Phase: <span className="capitalize">{error.phase}</span>
                      </span>
                    )}
                    {error.timestamp && (
                      <span>
                        {format(new Date(error.timestamp), 'MMM d, h:mm:ss a')}
                      </span>
                    )}
                  </div>
                </div>
              </CollapsibleContent>
            )}
          </div>
        </div>
      </div>
    </Collapsible>
  );
}

export type DocumentErrorSummaryProps = {
  errorCount: number;
  errorSummary: string;
  phase?: string | null;
};

export function DocumentErrorSummary({
  errorCount,
  errorSummary,
  phase,
}: DocumentErrorSummaryProps) {
  return (
    <DocumentErrorEntry
      error={{
        documentId: errorCount > 1 ? `${errorCount} documents failed` : undefined,
        message: errorSummary,
        phase,
      }}
    />
  );
}
