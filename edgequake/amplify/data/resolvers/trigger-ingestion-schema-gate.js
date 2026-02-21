/**
 * AppSync JS pipeline resolver (step 1 of 3): triggerIngestion - Schema Gate
 *
 * Reads the CONFIG record and verifies that a schema has been approved
 * (entity_types are populated). Blocks ingestion if no approved schema exists.
 *
 * This implements the user-locked decision: "require approved schema before
 * allowing ingestion."
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'GetItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.args.slug}`,
      SK: 'CONFIG',
    }),
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }

  if (!ctx.result) {
    return util.error('Namespace not found', 'NotFound');
  }

  const config = JSON.parse(ctx.result.data);

  // Schema gate: reject if no approved schema (empty entity_types in CONFIG)
  if (!config.entity_types || config.entity_types.length === 0) {
    return util.error(
      'No approved schema found. Approve a schema before triggering ingestion.',
      'SchemaNotApproved'
    );
  }

  // Stash slug for downstream steps
  ctx.stash.slug = ctx.args.slug;

  // Continue pipeline
  return null;
}
