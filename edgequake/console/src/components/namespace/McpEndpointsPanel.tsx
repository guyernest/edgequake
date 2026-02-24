import { useState } from 'react';
import { Copy, Check, Server, Database, Key, Wrench } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
} from '@/components/ui/card';
import { Skeleton } from '@/components/ui/skeleton';
import { toast } from 'sonner';
import { useNamespaceDescriptor } from '@/hooks/useNamespaceDetail';

function CopyableField({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      toast.success(`${label} copied`);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error('Failed to copy');
    }
  }

  return (
    <div className="flex items-start justify-between gap-2 rounded-md border bg-muted/50 px-3 py-2">
      <div className="min-w-0 flex-1">
        <p className="text-xs font-medium text-muted-foreground">{label}</p>
        <p className="mt-0.5 break-all font-mono text-sm">{value}</p>
      </div>
      <Button
        variant="ghost"
        size="icon-sm"
        className="mt-1 shrink-0"
        onClick={handleCopy}
      >
        {copied ? (
          <Check className="size-3.5" />
        ) : (
          <Copy className="size-3.5" />
        )}
      </Button>
    </div>
  );
}

export function McpEndpointsPanel({ slug }: { slug: string }) {
  const { data: descriptor, isLoading, error } = useNamespaceDescriptor(slug);

  if (isLoading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-48 w-full" />
        <Skeleton className="h-48 w-full" />
      </div>
    );
  }

  if (error) {
    return (
      <div className="rounded-lg border border-destructive/50 bg-destructive/10 p-4 text-sm text-destructive">
        Failed to load MCP descriptor: {error.message}
      </div>
    );
  }

  if (!descriptor) {
    return (
      <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-12 text-center">
        <Server className="mb-3 size-8 text-muted-foreground" />
        <h3 className="text-base font-semibold">No MCP Descriptor</h3>
        <p className="mt-1 max-w-md text-sm text-muted-foreground">
          The MCP descriptor is generated after a successful pipeline run.
          Run the batch pipeline to generate endpoint configuration.
        </p>
      </div>
    );
  }

  const storage = descriptor.storage as {
    neptune?: { endpoint?: string; label_prefix?: string };
    s3_vectors?: { bucket_name?: string; index_name?: string };
    dynamodb?: { table_name?: string; namespace_key?: string };
    bm25?: { database?: string; s3_bucket?: string; workgroup?: string };
  } | null;

  const auth = descriptor.auth as {
    role_arn?: string;
    external_id?: string;
    region?: string;
  } | null;

  const pipelineConfig = descriptor.pipeline_config as {
    embedding_model?: string;
    embedding_dimension?: number;
    llm_model?: string;
  } | null;

  const tools = descriptor.tools as
    | { name: string; description: string; modes?: string[] }[]
    | null;

  return (
    <div className="space-y-6">
      {/* Storage Endpoints */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <Database className="size-4" />
            Storage Endpoints
          </CardTitle>
          <CardDescription>
            Backend services for this namespace's knowledge graph
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {storage?.neptune?.endpoint && (
            <CopyableField
              label="Neptune Endpoint"
              value={storage.neptune.endpoint}
            />
          )}
          {storage?.neptune?.label_prefix && (
            <CopyableField
              label="Neptune Label Prefix"
              value={storage.neptune.label_prefix}
            />
          )}
          {storage?.s3_vectors?.bucket_name && (
            <CopyableField
              label="S3 Vectors Bucket"
              value={storage.s3_vectors.bucket_name}
            />
          )}
          {storage?.s3_vectors?.index_name && (
            <CopyableField
              label="S3 Vectors Index"
              value={storage.s3_vectors.index_name}
            />
          )}
          {storage?.dynamodb?.table_name && (
            <CopyableField
              label="DynamoDB Table"
              value={storage.dynamodb.table_name}
            />
          )}
          {storage?.dynamodb?.namespace_key && (
            <CopyableField
              label="DynamoDB Namespace Key"
              value={storage.dynamodb.namespace_key}
            />
          )}
          {storage?.bm25?.database && (
            <CopyableField
              label="BM25 Athena Database"
              value={storage.bm25.database}
            />
          )}
          {storage?.bm25?.s3_bucket && (
            <CopyableField
              label="BM25 S3 Bucket"
              value={storage.bm25.s3_bucket}
            />
          )}
          {storage?.bm25?.workgroup && (
            <CopyableField
              label="BM25 Athena Workgroup"
              value={storage.bm25.workgroup}
            />
          )}
        </CardContent>
      </Card>

      {/* Authentication */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <Key className="size-4" />
            Authentication
          </CardTitle>
          <CardDescription>
            IAM role for MCP server access to this namespace
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {auth?.role_arn && (
            <CopyableField label="Role ARN" value={auth.role_arn} />
          )}
          {auth?.external_id && (
            <CopyableField label="External ID" value={auth.external_id} />
          )}
          {auth?.region && (
            <CopyableField label="Region" value={auth.region} />
          )}
        </CardContent>
      </Card>

      {/* Tools */}
      {tools && tools.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-base">
              <Wrench className="size-4" />
              MCP Tools
            </CardTitle>
            <CardDescription>
              Tools available to MCP clients connecting to this namespace
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="space-y-2">
              {tools.map((tool) => (
                <div
                  key={tool.name}
                  className="rounded-md border px-3 py-2"
                >
                  <p className="font-mono text-sm font-medium">
                    {tool.name}
                  </p>
                  <p className="mt-0.5 text-xs text-muted-foreground">
                    {tool.description}
                  </p>
                  {tool.modes && (
                    <div className="mt-1.5 flex flex-wrap gap-1">
                      {tool.modes.map((mode) => (
                        <span
                          key={mode}
                          className="rounded-full bg-muted px-2 py-0.5 text-xs"
                        >
                          {mode}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}

      {/* Pipeline Config Snapshot */}
      {pipelineConfig && (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Pipeline Config Snapshot</CardTitle>
            <CardDescription>
              Model configuration at time of descriptor generation
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-3 gap-4 text-sm">
              {pipelineConfig.llm_model && (
                <div>
                  <p className="text-xs text-muted-foreground">LLM Model</p>
                  <p className="font-mono">{pipelineConfig.llm_model}</p>
                </div>
              )}
              {pipelineConfig.embedding_model && (
                <div>
                  <p className="text-xs text-muted-foreground">
                    Embedding Model
                  </p>
                  <p className="font-mono">
                    {pipelineConfig.embedding_model}
                  </p>
                </div>
              )}
              {pipelineConfig.embedding_dimension != null && (
                <div>
                  <p className="text-xs text-muted-foreground">
                    Vector Dimension
                  </p>
                  <p className="font-mono">
                    {pipelineConfig.embedding_dimension}
                  </p>
                </div>
              )}
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
