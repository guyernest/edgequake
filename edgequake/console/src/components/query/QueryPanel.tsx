import { useState } from 'react';
import { Search, Clock, Database, Zap, ChevronDown, ChevronRight } from 'lucide-react';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import {
  useNamespaceQuery,
  type NamespaceQueryResponse,
  type SourceReference,
  type QueryStats,
} from '@/hooks/useNamespaceQuery';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function sourceTypeBadge(type: string) {
  switch (type) {
    case 'chunk':
      return 'bg-blue-100 text-blue-800 dark:bg-blue-900/40 dark:text-blue-300';
    case 'entity':
      return 'bg-green-100 text-green-800 dark:bg-green-900/40 dark:text-green-300';
    case 'relationship':
      return 'bg-purple-100 text-purple-800 dark:bg-purple-900/40 dark:text-purple-300';
    default:
      return 'bg-gray-100 text-gray-800 dark:bg-gray-900/40 dark:text-gray-300';
  }
}

function formatMs(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

// ---------------------------------------------------------------------------
// Stats Display
// ---------------------------------------------------------------------------

function StatsBar({ stats, reranked }: { stats: QueryStats; reranked?: boolean }) {
  return (
    <div className="flex flex-wrap gap-3 rounded-md border bg-muted/30 px-4 py-2 text-xs">
      <span className="flex items-center gap-1">
        <Clock className="size-3" />
        Total: <strong>{formatMs(stats.total_time_ms)}</strong>
      </span>
      <span className="text-muted-foreground">|</span>
      <span>
        Embed: {formatMs(stats.embedding_time_ms)}
      </span>
      <span>
        Retrieve: {formatMs(stats.retrieval_time_ms)}
      </span>
      {stats.generation_time_ms > 0 && (
        <span>
          Generate: {formatMs(stats.generation_time_ms)}
        </span>
      )}
      {stats.rerank_time_ms != null && (
        <span>
          Rerank: {formatMs(stats.rerank_time_ms)}
        </span>
      )}
      <span className="text-muted-foreground">|</span>
      <span className="flex items-center gap-1">
        <Database className="size-3" />
        {stats.sources_retrieved} sources
      </span>
      {reranked && (
        <Badge variant="outline" className="text-[10px] px-1.5 py-0">
          reranked
        </Badge>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Source Card
// ---------------------------------------------------------------------------

function SourceCard({ source }: { source: SourceReference }) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="rounded-md border">
      <button
        type="button"
        className="flex w-full items-start gap-3 px-3 py-2 text-left hover:bg-muted/30"
        onClick={() => setExpanded(!expanded)}
      >
        <span className="mt-0.5">
          {expanded ? (
            <ChevronDown className="size-3.5" />
          ) : (
            <ChevronRight className="size-3.5" />
          )}
        </span>

        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <Badge variant="secondary" className={sourceTypeBadge(source.source_type)}>
              {source.source_type}
            </Badge>
            <span className="truncate text-sm font-medium">{source.id}</span>
            <span className="ml-auto shrink-0 text-xs text-muted-foreground tabular-nums">
              score: {source.score.toFixed(3)}
              {source.rerank_score != null && (
                <> / rerank: {source.rerank_score.toFixed(3)}</>
              )}
            </span>
          </div>

          {/* Metadata pills */}
          <div className="mt-1 flex flex-wrap gap-1.5 text-[11px] text-muted-foreground">
            {source.document_id && (
              <span className="rounded bg-muted px-1.5 py-0.5">
                doc: {source.document_id.slice(0, 16)}...
              </span>
            )}
            {source.chunk_index != null && (
              <span className="rounded bg-muted px-1.5 py-0.5">
                chunk #{source.chunk_index}
              </span>
            )}
            {source.start_line != null && source.end_line != null && (
              <span className="rounded bg-muted px-1.5 py-0.5">
                lines {source.start_line}-{source.end_line}
              </span>
            )}
            {source.content && (
              <span className="rounded bg-muted px-1.5 py-0.5">
                ~{Math.ceil(source.content.length / 4)} tokens
              </span>
            )}
            {source.reference_id != null && (
              <span className="rounded bg-muted px-1.5 py-0.5">
                ref [{source.reference_id}]
              </span>
            )}
          </div>
        </div>
      </button>

      {expanded && (source.content || source.snippet) && (
        <div className="border-t px-3 py-2">
          <pre className="whitespace-pre-wrap text-xs text-muted-foreground">
            {source.content ?? source.snippet}
          </pre>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Results Display
// ---------------------------------------------------------------------------

function ResultsDisplay({ data }: { data: NamespaceQueryResponse }) {
  const chunks = data.sources.filter((s) => s.source_type === 'chunk');
  const entities = data.sources.filter((s) => s.source_type === 'entity');
  const relationships = data.sources.filter((s) => s.source_type === 'relationship');

  return (
    <div className="space-y-4">
      <StatsBar stats={data.stats} reranked={data.reranked} />

      <div className="flex items-center gap-2 text-sm">
        <Badge variant="outline">mode: {data.mode}</Badge>
        {chunks.length > 0 && (
          <span className="text-muted-foreground">{chunks.length} chunks</span>
        )}
        {entities.length > 0 && (
          <span className="text-muted-foreground">{entities.length} entities</span>
        )}
        {relationships.length > 0 && (
          <span className="text-muted-foreground">{relationships.length} relationships</span>
        )}
      </div>

      {/* Answer (only shown if not context_only) */}
      {data.answer && (
        <div className="rounded-md border bg-muted/20 p-4">
          <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Answer
          </h4>
          <p className="text-sm whitespace-pre-wrap">{data.answer}</p>
        </div>
      )}

      {/* Chunks section */}
      {chunks.length > 0 && (
        <div>
          <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Chunks ({chunks.length})
          </h4>
          <div className="space-y-1.5">
            {chunks.map((s, i) => (
              <SourceCard key={`${s.id}-${i}`} source={s} />
            ))}
          </div>
        </div>
      )}

      {/* Entities section */}
      {entities.length > 0 && (
        <div>
          <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Entities ({entities.length})
          </h4>
          <div className="space-y-1.5">
            {entities.map((s, i) => (
              <SourceCard key={`${s.id}-${i}`} source={s} />
            ))}
          </div>
        </div>
      )}

      {/* Relationships section */}
      {relationships.length > 0 && (
        <div>
          <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Relationships ({relationships.length})
          </h4>
          <div className="space-y-1.5">
            {relationships.map((s, i) => (
              <SourceCard key={`${s.id}-${i}`} source={s} />
            ))}
          </div>
        </div>
      )}

      {data.sources.length === 0 && (
        <div className="rounded-lg border border-dashed p-8 text-center">
          <p className="text-sm text-muted-foreground">
            No sources retrieved for this query.
          </p>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// QueryPanel
// ---------------------------------------------------------------------------

interface QueryPanelProps {
  slug: string;
}

export function QueryPanel({ slug }: QueryPanelProps) {
  const [queryText, setQueryText] = useState('');
  const [mode, setMode] = useState('hybrid');
  const [retrievalMode, setRetrievalMode] = useState('vector');
  const [contextOnly, setContextOnly] = useState(true);
  const [enableRerank, setEnableRerank] = useState(true);

  const mutation = useNamespaceQuery(slug);

  const submitQuery = () => {
    if (!queryText.trim()) return;
    mutation.mutate({
      query: queryText.trim(),
      mode,
      retrieval_mode: retrievalMode,
      context_only: contextOnly,
      enable_rerank: enableRerank,
      include_references: true,
    });
  };

  return (
    <div className="space-y-4">
      {/* Query input row */}
      <div className="flex gap-3">
        <div className="relative flex-1">
          <Search className="absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            placeholder="Enter a query to search the knowledge graph..."
            value={queryText}
            onChange={(e) => setQueryText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submitQuery();
            }}
            className="pl-8"
          />
        </div>
        <Button onClick={submitQuery} disabled={mutation.isPending || !queryText.trim()}>
          <Zap className="mr-1.5 size-4" />
          {mutation.isPending ? 'Querying...' : 'Query'}
        </Button>
      </div>

      {/* Options row */}
      <div className="flex flex-wrap items-center gap-3">
        <div className="flex items-center gap-1.5">
          <label className="text-xs text-muted-foreground">Query mode:</label>
          <Select value={mode} onValueChange={setMode}>
            <SelectTrigger className="h-8 w-32 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="naive">Naive</SelectItem>
              <SelectItem value="local">Local</SelectItem>
              <SelectItem value="global">Global</SelectItem>
              <SelectItem value="hybrid">Hybrid</SelectItem>
              <SelectItem value="mix">Mix</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <div className="flex items-center gap-1.5">
          <label className="text-xs text-muted-foreground">Retrieval:</label>
          <Select value={retrievalMode} onValueChange={setRetrievalMode}>
            <SelectTrigger className="h-8 w-32 text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="vector">Vector</SelectItem>
              <SelectItem value="bm25">BM25</SelectItem>
              <SelectItem value="hybrid">Hybrid (RRF)</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <label className="flex items-center gap-1.5 text-xs">
          <input
            type="checkbox"
            checked={contextOnly}
            onChange={(e) => setContextOnly(e.target.checked)}
            className="rounded"
          />
          Context only (skip LLM)
        </label>

        <label className="flex items-center gap-1.5 text-xs">
          <input
            type="checkbox"
            checked={enableRerank}
            onChange={(e) => setEnableRerank(e.target.checked)}
            className="rounded"
          />
          Rerank
        </label>
      </div>

      {/* Error state */}
      {mutation.isError && (
        <div className="rounded-md border border-red-200 bg-red-50 p-4 dark:border-red-800 dark:bg-red-950">
          <p className="text-sm text-red-700 dark:text-red-400">
            Query failed
            {mutation.error instanceof Error ? `: ${mutation.error.message}` : '.'}
          </p>
        </div>
      )}

      {/* Results */}
      {mutation.data && <ResultsDisplay data={mutation.data} />}
    </div>
  );
}
