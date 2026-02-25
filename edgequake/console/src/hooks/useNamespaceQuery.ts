import { useMutation } from '@tanstack/react-query';
import { edgequakeApi } from '@/lib/api-client';

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

export interface NamespaceQueryRequest {
  query: string;
  mode?: string;
  context_only?: boolean;
  include_references?: boolean;
  enable_rerank?: boolean;
  rerank_top_k?: number;
  retrieval_mode?: string; // "vector" | "bm25" | "hybrid"
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

export interface SourceReference {
  source_type: string;
  id: string;
  score: number;
  rerank_score?: number;
  snippet?: string;
  content?: string;
  reference_id?: number;
  document_id?: string;
  file_path?: string;
  start_line?: number;
  end_line?: number;
  chunk_index?: number;
}

export interface QueryStats {
  embedding_time_ms: number;
  retrieval_time_ms: number;
  generation_time_ms: number;
  total_time_ms: number;
  sources_retrieved: number;
  rerank_time_ms?: number;
  tokens_used?: number;
  tokens_per_second?: number;
  llm_provider?: string;
  llm_model?: string;
}

export interface NamespaceQueryResponse {
  answer: string;
  mode: string;
  sources: SourceReference[];
  stats: QueryStats;
  conversation_id?: string;
  reranked?: boolean;
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

export function useNamespaceQuery(slug: string) {
  return useMutation({
    mutationFn: (req: NamespaceQueryRequest) =>
      edgequakeApi.post<NamespaceQueryResponse>(
        `/api/v1/ns/${slug}/query`,
        req,
      ),
  });
}
