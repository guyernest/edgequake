/**
 * Step 3 of 3: getPreviewResult - Combine request + result data
 *
 * Returns the full preview result if completed, status/progress if still
 * running, or null if no preview has been requested.
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  // No-op read — we only need the response handler to combine stash data
  return {
    operation: 'GetItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.stash.slug}`,
      SK: 'PREVIEW_REQUEST',
    }),
  };
}

export function response(ctx) {
  const previewResult = ctx.stash.previewResult || null;
  const previewRequest = ctx.stash.previewRequest || null;

  // If we have a full result, return it
  if (previewResult && (previewResult.status === 'completed' || previewResult.status === 'cancelled')) {
    return previewResult;
  }

  // If we have only a request, return status/progress info
  if (previewRequest) {
    return {
      status: previewRequest.status,
      entityTypeCounts: null,
      relationTypeCounts: null,
      coverageRows: null,
      documentColumns: null,
      totalChunks: null,
      totalEntities: null,
      totalRelationships: null,
      cost: null,
      processingTimeMs: null,
      documentsCompleted: previewRequest.documents_completed || 0,
      documentsTotal: previewRequest.documents_total || 3,
    };
  }

  // No preview data at all
  return null;
}
