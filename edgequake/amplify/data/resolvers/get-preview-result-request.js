/**
 * Step 1 of 3: getPreviewResult - Read PREVIEW_REQUEST record
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'GetItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.args.namespace}`,
      SK: 'PREVIEW_REQUEST',
    }),
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }
  if (ctx.result) {
    ctx.stash.previewRequest = JSON.parse(ctx.result.data);
  }
  ctx.stash.slug = ctx.args.namespace;
  return {};
}
