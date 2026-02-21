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
import { useNamespace, useSchemaProposal } from '@/hooks/useNamespaceDetail';
import { usePipelineConfig } from '@/hooks/usePipelineConfig';
import { useNamespaceStatus } from '@/hooks/useNamespaceStatus';

function SchemaTabContent({ slug }: { slug: string }) {
  const { data: schema, isLoading } = useSchemaProposal(slug);

  if (isLoading) {
    return (
      <div className="space-y-3 p-4">
        <Skeleton className="h-5 w-40" />
        <Skeleton className="h-4 w-64" />
        <Skeleton className="h-4 w-48" />
      </div>
    );
  }

  if (!schema) {
    return (
      <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-12 text-center">
        <h3 className="text-base font-semibold">No schema proposal</h3>
        <p className="mt-1 text-sm text-muted-foreground">
          Run the pipeline to generate a schema proposal from your documents.
        </p>
      </div>
    );
  }

  const entityTypes = Array.isArray(schema.entity_types)
    ? schema.entity_types
    : [];
  const relationTypes = Array.isArray(schema.relation_types)
    ? schema.relation_types
    : [];

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <span className="text-sm font-medium">Status:</span>
        <span className="rounded-full bg-muted px-2.5 py-0.5 text-xs font-medium capitalize">
          {schema.status}
        </span>
      </div>
      <div className="grid grid-cols-2 gap-4">
        <div className="rounded-md border p-4">
          <div className="text-sm font-medium">Entity Types</div>
          <div className="mt-1 text-2xl font-bold">{entityTypes.length}</div>
        </div>
        <div className="rounded-md border p-4">
          <div className="text-sm font-medium">Relation Types</div>
          <div className="mt-1 text-2xl font-bold">{relationTypes.length}</div>
        </div>
      </div>
      {schema.sample_size != null && (
        <p className="text-xs text-muted-foreground">
          Proposed from {schema.sample_size} of {schema.total_documents} documents
        </p>
      )}
    </div>
  );
}

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
          <SchemaTabContent slug={slug} />
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
