import { useNavigate } from 'react-router-dom';
import { formatDistanceToNow } from 'date-fns';
import { Database, GitFork, FileText, Brain } from 'lucide-react';
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
} from '@/components/ui/card';
import { NamespaceStatusBadge } from '@/components/namespace/NamespaceStatusBadge';
import { useNamespaceStatus } from '@/hooks/useNamespaceStatus';
import { usePipelineConfig } from '@/hooks/usePipelineConfig';

type NamespaceCardProps = {
  slug: string;
  description?: string | null;
  createdAt: number;
};

export function NamespaceCard({
  slug,
  description,
  createdAt,
}: NamespaceCardProps) {
  const navigate = useNavigate();
  const { data: status } = useNamespaceStatus(slug);
  const { data: config } = usePipelineConfig(slug);

  const lastRunTimestamp = status?.updated_at ?? status?.started_at;
  const lastRunLabel = lastRunTimestamp
    ? formatDistanceToNow(new Date(lastRunTimestamp), {
        addSuffix: true,
      })
    : null;

  const createdLabel = formatDistanceToNow(new Date(createdAt), {
    addSuffix: true,
  });

  return (
    <Card
      className="cursor-pointer transition-shadow hover:shadow-lg"
      onClick={() => {
        void navigate(`/ns/${slug}`);
      }}
    >
      <CardHeader>
        <div className="flex items-center justify-between">
          <CardTitle className="text-lg">{slug}</CardTitle>
          <NamespaceStatusBadge status={status?.status} />
        </div>
        {description && (
          <CardDescription className="line-clamp-2">
            {description}
          </CardDescription>
        )}
      </CardHeader>

      <CardContent>
        <div className="grid grid-cols-2 gap-3 text-sm">
          <div className="flex items-center gap-2 text-muted-foreground">
            <Brain className="size-4 shrink-0" />
            <span className="truncate">
              {config?.llm_model ?? 'Not configured'}
            </span>
          </div>

          <div className="flex items-center gap-2 text-muted-foreground">
            <Database className="size-4 shrink-0" />
            <span>
              {config?.entity_types
                ? `${config.entity_types.length} types`
                : 'No schema'}
            </span>
          </div>

          {status?.total_documents != null && (
            <div className="flex items-center gap-2 text-muted-foreground">
              <FileText className="size-4 shrink-0" />
              <span>{status.total_documents} docs</span>
            </div>
          )}

          {status?.total_chunks != null && (
            <div className="flex items-center gap-2 text-muted-foreground">
              <GitFork className="size-4 shrink-0" />
              <span>{status.total_chunks} chunks</span>
            </div>
          )}
        </div>
      </CardContent>

      <CardFooter className="text-xs text-muted-foreground">
        <div className="flex w-full justify-between">
          <span>Created {createdLabel}</span>
          {lastRunLabel && <span>Last run {lastRunLabel}</span>}
        </div>
      </CardFooter>
    </Card>
  );
}
