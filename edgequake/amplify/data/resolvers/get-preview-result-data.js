/**
 * Step 2 of 3: getPreviewResult - Read PREVIEW_RESULT record
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'GetItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.stash.slug}`,
      SK: 'PREVIEW_RESULT',
    }),
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }
  if (ctx.result) {
    ctx.stash.previewResult = JSON.parse(ctx.result.data);
  }
  return {};
}
