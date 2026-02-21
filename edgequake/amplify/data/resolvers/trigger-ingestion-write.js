/**
 * AppSync JS pipeline resolver (step 2 of 2): triggerIngestion - Write
 *
 * Writes the RUN_REQUEST record to DynamoDB.
 * Uses the validated data from ctx.stash (set by step 1).
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'PutItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.stash.slug}`,
      SK: 'RUN_REQUEST',
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
