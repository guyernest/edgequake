/**
 * AppSync JS pipeline resolver (step 2 of 2): updatePipelineConfig - Write
 *
 * Writes the merged pipeline config back to DynamoDB.
 * Uses the validated data from ctx.stash (set by step 1).
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'PutItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.stash.slug}`,
      SK: 'CONFIG',
    }),
    attributeValues: util.dynamodb.toMapValues({
      data: ctx.stash.updatedData,
    }),
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }

  // Return the merged config (from previous pipeline step)
  return ctx.prev.result;
}
