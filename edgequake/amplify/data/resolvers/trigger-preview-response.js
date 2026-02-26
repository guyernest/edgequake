/**
 * AppSync JS pipeline resolver (step 2 of 2): triggerPreviewExtraction - Write
 *
 * Writes the PREVIEW_REQUEST record to DynamoDB with status="requested"
 * so the UI sees the request immediately. The operator then runs the CLI
 * command to execute the preview.
 *
 * Uses the validated data from ctx.stash (set by step 1).
 * Follows the same pattern as trigger-ingestion-write.js.
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'PutItem',
    key: util.dynamodb.toMapValues({
      PK: 'NS#' + ctx.stash.slug,
      SK: 'PREVIEW_REQUEST',
    }),
    attributeValues: util.dynamodb.toMapValues({
      data: ctx.stash.previewRequest,
    }),
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }

  // Return the preview status (from previous pipeline step)
  return ctx.prev.result;
}
