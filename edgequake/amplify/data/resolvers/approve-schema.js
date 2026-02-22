/**
 * AppSync JS pipeline resolver (step 1 of 2): approveSchema - Read
 *
 * Gets the SCHEMA record and validates it is in "proposed" state.
 * Passes the validated/updated proposal to step 2 via ctx.stash.
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'GetItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.args.slug}`,
      SK: 'SCHEMA',
    }),
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }

  if (!ctx.result) {
    return util.error(
      'Schema not found for this namespace. Run schema suggestion first.',
      'NotFound'
    );
  }

  const data = JSON.parse(ctx.result.data);

  if (data.status !== 'proposed') {
    return util.error(
      `Schema is not in 'proposed' state (current: ${data.status}). Only proposed schemas can be approved.`,
      'InvalidSchemaState'
    );
  }

  // Update status and reviewed_at
  data.status = 'approved';
  data.reviewed_at = Math.floor(util.time.nowEpochSeconds());

  // Pass updated data to next pipeline step via stash
  ctx.stash.updatedData = JSON.stringify(data);
  ctx.stash.slug = ctx.args.slug;

  return data;
}
