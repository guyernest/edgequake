/**
 * AppSync JS resolver: getNamespaceStatus
 *
 * Reads pipeline status from DynamoDB using PK=NS#{slug}, SK=LATEST_RUN.
 * Returns the parsed run status object, or a default idle status if no run exists.
 *
 * Status fields: status, phase, job_id, total_documents, processed_documents,
 * total_chunks, current_batch, total_batches, started_at, updated_at, error_summary
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'GetItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.args.slug}`,
      SK: 'LATEST_RUN',
    }),
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }

  if (!ctx.result) {
    return {
      status: 'idle',
      phase: null,
      job_id: null,
      total_documents: null,
      processed_documents: null,
      total_chunks: null,
      current_batch: null,
      total_batches: null,
      started_at: null,
      updated_at: null,
      error_summary: null,
    };
  }

  return JSON.parse(ctx.result.data);
}
