/**
 * AppSync JS pipeline resolver (step 1 of 2): updatePipelineConfig - Read
 *
 * Gets the CONFIG record for a namespace, merges the provided config updates
 * with the existing config, and passes the merged result to step 2 via ctx.stash.
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
    return util.error(
      'Pipeline config not found for this namespace.',
      'NotFound'
    );
  }

  const existing = JSON.parse(ctx.result.data);
  const updates = ctx.args.config;

  // Merge updates into existing config
  const merged = { ...existing, ...updates };
  merged.updated_at = Math.floor(util.time.nowEpochSeconds());

  // Pass merged result to next pipeline step via stash
  ctx.stash.updatedData = JSON.stringify(merged);
  ctx.stash.slug = ctx.args.slug;

  return merged;
}
