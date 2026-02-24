import { useParams, Link } from 'react-router-dom';
import { ArrowLeft } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { Skeleton } from '@/components/ui/skeleton';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs';
import { NamespaceStatusBadge } from '@/components/namespace/NamespaceStatusBadge';
import { LlmConfigForm } from '@/components/config/LlmConfigForm';
import { ExtractionConfigForm } from '@/components/config/ExtractionConfigForm';
import { PipelineTimeline } from '@/components/pipeline/PipelineTimeline';
import { TriggerIngestionDialog } from '@/components/ingestion/TriggerIngestionDialog';
import { SchemaEditor } from '@/components/schema/SchemaEditor';
import { McpEndpointsPanel } from '@/components/namespace/McpEndpointsPanel';
import { useNamespace } from '@/hooks/useNamespaceDetail';
import { usePipelineConfig } from '@/hooks/usePipelineConfig';
import { useNamespaceStatus } from '@/hooks/useNamespaceStatus';

export function NamespacePage() {
  const { namespace: slug } = useParams<{ namespace: string }>();

  const {
    data: namespace,
    isLoading: nsLoading,
    error: nsError,
  } = useNamespace(slug ?? '');

  const { data: status } = useNamespaceStatus(slug ?? '');
  const { data: pipelineConfig } = usePipelineConfig(slug ?? '');

  if (!slug) {
    return (
      <div className="p-8 text-center text-muted-foreground">
        Missing namespace parameter.
      </div>
    );
  }

  return (
    <div className="p-6">
      {/* Header */}
      <div className="mb-6">
        <Link to="/">
          <Button variant="ghost" size="sm" className="mb-3 -ml-2 gap-1">
            <ArrowLeft className="size-4" />
            Back to Dashboard
          </Button>
        </Link>

        {nsLoading ? (
          <div className="flex items-center gap-3">
            <Skeleton className="h-7 w-48" />
            <Skeleton className="h-5 w-16 rounded-full" />
          </div>
        ) : nsError ? (
          <div className="rounded-lg border border-destructive/50 bg-destructive/10 p-4 text-sm text-destructive">
            Failed to load namespace: {nsError.message}
          </div>
        ) : (
          <div className="flex items-center gap-3">
            <h1 className="text-2xl font-bold tracking-tight">
              {namespace?.slug ?? slug}
            </h1>
            <NamespaceStatusBadge status={status?.status} />
            <div className="ml-auto">
              <TriggerIngestionDialog
                slug={slug}
                pipelineStatus={status?.status}
                llmModel={pipelineConfig?.llm_model ?? undefined}
                embeddingModel={pipelineConfig?.embedding_model ?? undefined}
              />
            </div>
          </div>
        )}

        {namespace?.description && (
          <p className="mt-1 text-sm text-muted-foreground">
            {namespace.description}
          </p>
        )}
      </div>

      {/* Tabs */}
      <Tabs defaultValue="status">
        <TabsList>
          <TabsTrigger value="status">Status</TabsTrigger>
          <TabsTrigger value="configuration">Configuration</TabsTrigger>
          <TabsTrigger value="schema">Schema</TabsTrigger>
          <TabsTrigger value="endpoints">Endpoints</TabsTrigger>
          <TabsTrigger value="history">Run History</TabsTrigger>
        </TabsList>

        <TabsContent value="status" className="mt-4">
          <PipelineTimeline status={status} />
        </TabsContent>

        <TabsContent value="configuration" className="mt-4">
          <div className="space-y-8">
            <LlmConfigForm slug={slug} />
            <Separator />
            <ExtractionConfigForm slug={slug} />
          </div>
        </TabsContent>

        <TabsContent value="schema" className="mt-4">
          <SchemaEditor slug={slug} />
        </TabsContent>

        <TabsContent value="endpoints" className="mt-4">
          <McpEndpointsPanel slug={slug} />
        </TabsContent>

        <TabsContent value="history" className="mt-4">
          <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-12 text-center">
            <h3 className="text-base font-semibold">Run History</h3>
            <p className="mt-1 text-sm text-muted-foreground">
              Run history deferred to future phase
            </p>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  );
}
