/**
 * AppSync JS pipeline resolver (step 1 of 2): getPreviewResult - Read
 *
 * Reads PREVIEW_RESULT and PREVIEW_REQUEST from DynamoDB to get the
 * preview status and results. Uses BatchGetItem to read both in one call.
 *
 * - If PREVIEW_RESULT exists with status="completed": return full results
 * - If only PREVIEW_REQUEST exists: return status + progress
 * - If neither exists: return null
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  var table = ctx.env.NAMESPACE_TABLE;
  var slug = ctx.args.namespace;
  return {
    operation: 'BatchGetItem',
    tables: {
      [table]: {
        keys: [
          util.dynamodb.toMapValues({ PK: 'NS#' + slug, SK: 'PREVIEW_RESULT' }),
          util.dynamodb.toMapValues({ PK: 'NS#' + slug, SK: 'PREVIEW_REQUEST' }),
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
  var previewResult = null;
  var previewRequest = null;

  for (var i = 0; i < items.length; i++) {
    if (items[i].SK === 'PREVIEW_RESULT') {
      previewResult = JSON.parse(items[i].data);
    } else if (items[i].SK === 'PREVIEW_REQUEST') {
      previewRequest = JSON.parse(items[i].data);
    }
  }

  // If we have a full result, return it
  if (previewResult && (previewResult.status === 'completed' || previewResult.status === 'cancelled')) {
    ctx.stash.previewData = previewResult;
    return previewResult;
  }

  // If we have only a request, return status/progress info
  if (previewRequest) {
    var statusResult = {
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
    ctx.stash.previewData = statusResult;
    return statusResult;
  }

  // No preview data at all
  return null;
}
