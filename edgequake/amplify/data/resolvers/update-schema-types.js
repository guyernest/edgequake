/**
 * AppSync JS pipeline resolver (step 1 of 2): updateSchemaTypes - Read
 *
 * Gets the SCHEMA record and validates it is in "proposed" state.
 * Merges provided entity_types and/or relation_types into the proposal.
 * Passes the updated proposal to step 2 via ctx.stash.
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

  if (data.status !== 'proposed' && data.status !== 'approved') {
    return util.error(
      `Schema can only be edited in 'proposed' or 'approved' state (current: ${data.status}). Run suggest-schema again to create a new proposal.`,
      'InvalidSchemaState'
    );
  }

  // Replace entity_types if provided
  if (ctx.args.entity_types !== null && ctx.args.entity_types !== undefined) {
    data.entity_types = ctx.args.entity_types;
  }

  // Replace relation_types if provided
  if (ctx.args.relation_types !== null && ctx.args.relation_types !== undefined) {
    data.relation_types = ctx.args.relation_types;
  }

  // Editing an approved schema resets to proposed (requires re-approval)
  if (data.status === 'approved') {
    data.status = 'proposed';
    data.reviewed_at = null;
  }

  // Pass updated data to next pipeline step via stash
  ctx.stash.updatedData = JSON.stringify(data);
  ctx.stash.slug = ctx.args.slug;

  return data;
}
