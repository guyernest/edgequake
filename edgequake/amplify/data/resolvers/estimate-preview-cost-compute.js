/**
 * Step 3 of 3: estimatePreviewCost - Compute cost estimate
 *
 * Uses CONFIG and SCHEMA data from stash to compute a rough cost estimate.
 * Uses standard API rates (not batch rates).
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  // No-op read — we only need the response handler for computation
  return {
    operation: 'GetItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.stash.slug}`,
      SK: 'CONFIG',
    }),
  };
}

export function response(ctx) {
  const config = ctx.stash.config;

  // Estimate: 3 documents, ~13 chunks each (512-token chunks), capped at 60
  const docCount = 3;
  const estimatedChunksPerDoc = 13;
  const totalChunks = Math.min(docCount * estimatedChunksPerDoc, 60);

  // Token estimation: ~1500 input tokens per chunk, ~500 output tokens per chunk
  const inputTokensPerChunk = 1500;
  const outputTokensPerChunk = 500;
  const estimatedInputTokens = totalChunks * inputTokensPerChunk;
  const estimatedOutputTokens = totalChunks * outputTokensPerChunk;

  // Model from config (default to gpt-4.1-nano)
  const model = config.llm_model || 'gpt-4.1-nano';

  // Standard pricing per 1K tokens (not batch rates)
  const rates = { input: 0.00015, output: 0.0006 };
  if (model === 'gpt-4o') {
    rates.input = 0.0025;
    rates.output = 0.01;
  } else if (model === 'gpt-4o-mini' || model === 'gpt-4.1-mini') {
    rates.input = 0.0004;
    rates.output = 0.0016;
  }

  const estimatedCostUsd = (estimatedInputTokens / 1000) * rates.input + (estimatedOutputTokens / 1000) * rates.output;

  // Build placeholder document list (real selection happens in CLI)
  const documents = [1, 2, 3].map((n) => ({
    id: `doc-${n}`,
    name: `Document ${n} (selected at runtime)`,
    chunkCount: Math.floor(totalChunks / docCount),
  }));

  return {
    documentCount: docCount,
    chunkCount: totalChunks,
    estimatedInputTokens: estimatedInputTokens,
    estimatedOutputTokens: estimatedOutputTokens,
    estimatedCostUsd: Math.round(estimatedCostUsd * 10000) / 10000,
    model: model,
    documents: documents,
  };
}
