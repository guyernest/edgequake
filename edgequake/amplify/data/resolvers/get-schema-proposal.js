/**
 * AppSync JS resolver: getSchemaProposal
 *
 * Reads the SCHEMA record for a namespace from DynamoDB.
 * Returns the parsed schema proposal data, or null if not found.
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
  if (!ctx.result) return null;
  return JSON.parse(ctx.result.data);
}
