/**
 * AppSync JS pipeline resolver (step 3 of 4): approveSchema - Read CONFIG
 *
 * Reads the PipelineConfig CONFIG record and merges entity/relation type names
 * from the approved schema using additive Set union (accumulates across
 * multiple approvals).
 *
 * Uses ctx.stash.slug (set by step 1) and ctx.stash.updatedData (the approved schema).
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'GetItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.stash.slug}`,
      SK: 'CONFIG',
    }),
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }

  // Parse existing CONFIG (or start with empty object if none exists)
  const config = ctx.result ? JSON.parse(ctx.result.data) : {};

  // Parse the approved schema from stash
  const schema = JSON.parse(ctx.stash.updatedData);

  // Extract new entity and relation type names from the schema
  const newEntityTypes = (schema.entity_types || []).map((e) => e.name);
  const newRelationTypes = (schema.relation_types || []).map((r) => r.name);

  // Additive merge using Set union (accumulates across multiple approvals)
  config.entity_types = [
    ...new Set([...(config.entity_types || []), ...newEntityTypes]),
  ];
  config.relation_types = [
    ...new Set([...(config.relation_types || []), ...newRelationTypes]),
  ];

  config.updated_at = util.time.nowEpochMilliSeconds();

  // Stash merged config for step 4 to write
  ctx.stash.configData = JSON.stringify(config);

  // Pass through the schema proposal result
  return ctx.prev.result;
}
