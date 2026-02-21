/**
 * AppSync JS pipeline resolver (step 4 of 4): approveSchema - Write CONFIG
 *
 * Writes the merged PipelineConfig CONFIG record back to DynamoDB.
 * Uses ctx.stash.configData (set by step 3) containing the additively merged
 * entity_types and relation_types.
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
      data: ctx.stash.configData,
    }),
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }

  // Return the schema proposal (the final response to the client)
  return ctx.prev.result;
}
