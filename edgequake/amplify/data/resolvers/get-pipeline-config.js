/**
 * AppSync JS resolver: getNamespacePipelineConfig
 *
 * Gets pipeline configuration from DynamoDB using PK=NS#{slug}, SK=CONFIG.
 * The data attribute is JSON-stringified (per Rust DynamoNamespaceRegistry pattern).
 * Returns parsed pipeline config object, or null if not found.
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
    return null;
  }

  return JSON.parse(ctx.result.data);
}
