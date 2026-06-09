/**
 * Step 1 of 3: estimatePreviewCost - Read CONFIG record
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'GetItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.args.namespace}`,
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
  ctx.stash.config = JSON.parse(ctx.result.data);
  ctx.stash.slug = ctx.args.namespace;
  return {};
}
