/**
 * Step 2 of 3: estimatePreviewCost - Read SCHEMA record
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'GetItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.stash.slug}`,
      SK: 'SCHEMA',
    }),
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }
  if (!ctx.result) {
    return util.error('No schema found. Create and approve a schema before previewing.', 'SchemaNotApproved');
  }
  const schema = JSON.parse(ctx.result.data);
  if (schema.status !== 'approved' && schema.status !== 'proposed') {
    return util.error('Schema must be approved or proposed before previewing.', 'SchemaNotApproved');
  }
  ctx.stash.schema = schema;
  return {};
}
