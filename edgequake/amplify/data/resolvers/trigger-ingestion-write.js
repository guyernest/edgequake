/**
 * AppSync JS pipeline resolver (step 3 of 3): triggerIngestion - Write
 *
 * Writes the LATEST_RUN record to DynamoDB so the UI sees "Queued" status
 * immediately. The previous version wrote to a different SK which was invisible
 * to the getNamespaceStatus query (which reads SK=LATEST_RUN).
 *
 * Uses the validated data from ctx.stash (set by step 2).
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'PutItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.stash.slug}`,
      SK: 'LATEST_RUN',
    }),
    attributeValues: util.dynamodb.toMapValues({
      data: ctx.stash.runRequest,
    }),
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }

  // Return the ingestion request (from previous pipeline step)
  return ctx.prev.result;
}
