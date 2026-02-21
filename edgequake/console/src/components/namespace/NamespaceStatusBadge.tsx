import { Badge } from '@/components/ui/badge';
import { cn } from '@/lib/utils';

type StatusBadgeProps = {
  status: string | null | undefined;
};

const STATUS_CONFIG: Record<
  string,
  { label: string; className: string; pulse?: boolean }
> = {
  idle: {
    label: 'Idle',
    className: 'bg-muted text-muted-foreground border-muted-foreground/20',
  },
  running: {
    label: 'Running',
    className: 'bg-blue-100 text-blue-800 border-blue-200 dark:bg-blue-900/40 dark:text-blue-300 dark:border-blue-800',
    pulse: true,
  },
  preparing: {
    label: 'Preparing',
    className: 'bg-blue-100 text-blue-800 border-blue-200 dark:bg-blue-900/40 dark:text-blue-300 dark:border-blue-800',
    pulse: true,
  },
  extracting: {
    label: 'Extracting',
    className: 'bg-blue-100 text-blue-800 border-blue-200 dark:bg-blue-900/40 dark:text-blue-300 dark:border-blue-800',
    pulse: true,
  },
  embedding: {
    label: 'Embedding',
    className: 'bg-blue-100 text-blue-800 border-blue-200 dark:bg-blue-900/40 dark:text-blue-300 dark:border-blue-800',
    pulse: true,
  },
  storing: {
    label: 'Storing',
    className: 'bg-blue-100 text-blue-800 border-blue-200 dark:bg-blue-900/40 dark:text-blue-300 dark:border-blue-800',
    pulse: true,
  },
  completed: {
    label: 'Completed',
    className: 'bg-green-100 text-green-800 border-green-200 dark:bg-green-900/40 dark:text-green-300 dark:border-green-800',
  },
  completed_with_warnings: {
    label: 'Completed with warnings',
    className: 'bg-amber-100 text-amber-800 border-amber-200 dark:bg-amber-900/40 dark:text-amber-300 dark:border-amber-800',
  },
  failed: {
    label: 'Error',
    className: 'bg-red-100 text-red-800 border-red-200 dark:bg-red-900/40 dark:text-red-300 dark:border-red-800',
  },
  error: {
    label: 'Error',
    className: 'bg-red-100 text-red-800 border-red-200 dark:bg-red-900/40 dark:text-red-300 dark:border-red-800',
  },
  requested: {
    label: 'Queued',
    className: 'bg-yellow-100 text-yellow-800 border-yellow-200 dark:bg-yellow-900/40 dark:text-yellow-300 dark:border-yellow-800',
  },
};

const DEFAULT_CONFIG = STATUS_CONFIG.idle!;

export function NamespaceStatusBadge({ status }: StatusBadgeProps) {
  const config = (status ? STATUS_CONFIG[status] : undefined) ?? DEFAULT_CONFIG;

  return (
    <Badge
      variant="outline"
      className={cn(config.className, config.pulse && 'animate-pulse')}
    >
      {config.label}
    </Badge>
  );
}
