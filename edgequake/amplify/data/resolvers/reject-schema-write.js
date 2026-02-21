/**
 * AppSync JS pipeline resolver (step 2 of 2): rejectSchema - Write
 *
 * Writes the updated SCHEMA record back to DynamoDB with status "rejected".
 * Uses the validated data from ctx.stash (set by step 1).
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'PutItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.stash.slug}`,
      SK: 'SCHEMA',
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

  // Return the updated proposal (from previous pipeline step)
  return ctx.prev.result;
}
