/**
 * AppSync JS resolver: getNamespaceDescriptor
 *
 * Reads MCP descriptor from DynamoDB using PK=NS#{slug}, SK=DESCRIPTOR.
 * Returns the parsed McpDescriptor object, or null if no descriptor exists.
 *
 * The stored JSON matches the Rust McpDescriptor struct directly —
 * no field mapping needed.
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'GetItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.args.slug}`,
      SK: 'DESCRIPTOR',
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
