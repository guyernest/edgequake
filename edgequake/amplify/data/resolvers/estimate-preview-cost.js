/**
 * AppSync JS pipeline resolver (step 1 of 2): estimatePreviewCost - Read
 *
 * Reads the CONFIG and SCHEMA records for a namespace from DynamoDB.
 * Computes a rough cost estimate based on document count, chunk estimation,
 * and model pricing. Uses standard API rates (not batch rates).
 *
 * Step 2 (estimate-preview-cost-response.js) shapes the final response.
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  // BatchGetItem to read both CONFIG and SCHEMA in one call
  var table = ctx.env.NAMESPACE_TABLE;
  var slug = ctx.args.namespace;
  return {
    operation: 'BatchGetItem',
    tables: {
      [table]: {
        keys: [
          util.dynamodb.toMapValues({ PK: 'NS#' + slug, SK: 'CONFIG' }),
          util.dynamodb.toMapValues({ PK: 'NS#' + slug, SK: 'SCHEMA' }),
        ],
      },
    },
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }

  var table = ctx.env.NAMESPACE_TABLE;
  var items = ctx.result.data[table] || [];
  var config = null;
  var schema = null;

  for (var i = 0; i < items.length; i++) {
    if (items[i].SK === 'CONFIG') {
      config = JSON.parse(items[i].data);
    } else if (items[i].SK === 'SCHEMA') {
      schema = JSON.parse(items[i].data);
    }
  }

  if (!config) {
    return util.error('Namespace not found', 'NotFound');
  }

  if (!schema || schema.status !== 'approved') {
    return util.error('No approved schema found. Approve a schema before previewing.', 'SchemaNotApproved');
  }

  // Count entity types from the config (set during schema approval)
  var entityTypes = config.entity_types || [];
  var entityTypeCount = entityTypes.length;

  // Estimate: 3 documents, ~6 chunks each (typical), capped at 20
  var docCount = 3;
  var estimatedChunksPerDoc = 6;
  var totalChunks = docCount * estimatedChunksPerDoc;
  if (totalChunks > 20) {
    totalChunks = 20;
  }

  // Token estimation: ~1500 input tokens per chunk (system prompt + content),
  // ~500 output tokens per chunk
  var inputTokensPerChunk = 1500;
  var outputTokensPerChunk = 500;
  var estimatedInputTokens = totalChunks * inputTokensPerChunk;
  var estimatedOutputTokens = totalChunks * outputTokensPerChunk;

  // Model from config (default to gpt-4.1-nano)
  var model = config.llm_model || 'gpt-4.1-nano';

  // Standard pricing per 1K tokens (not batch rates)
  var inputRate = 0.00015; // gpt-4.1-nano default
  var outputRate = 0.0006;
  if (model === 'gpt-4o') {
    inputRate = 0.0025;
    outputRate = 0.01;
  } else if (model === 'gpt-4o-mini' || model === 'gpt-4.1-mini') {
    inputRate = 0.0004;
    outputRate = 0.0016;
  }

  var estimatedCostUsd = (estimatedInputTokens / 1000) * inputRate + (estimatedOutputTokens / 1000) * outputRate;

  // Build placeholder document list (real selection happens in CLI)
  var documents = [];
  for (var d = 0; d < docCount; d++) {
    documents.push({
      id: 'doc-' + (d + 1),
      name: 'Document ' + (d + 1) + ' (selected at runtime)',
      chunkCount: Math.floor(totalChunks / docCount),
    });
  }

  // Stash for response shaping
  ctx.stash.costEstimate = {
    documentCount: docCount,
    chunkCount: totalChunks,
    estimatedInputTokens: estimatedInputTokens,
    estimatedOutputTokens: estimatedOutputTokens,
    estimatedCostUsd: Math.round(estimatedCostUsd * 10000) / 10000,
    model: model,
    documents: documents,
  };

  return ctx.stash.costEstimate;
}
