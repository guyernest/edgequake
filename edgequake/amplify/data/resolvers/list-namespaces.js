/**
 * AppSync JS resolver: listNamespaces
 *
 * Queries DynamoDB for all namespace entries under PK=NAMESPACES.
 * Each item has SK={slug} plus optional metadata fields.
 * Returns an array of Namespace objects.
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'Query',
    query: {
      expression: 'PK = :pk',
      expressionValues: util.dynamodb.toMapValues({
        ':pk': 'NAMESPACES',
      }),
    },
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }

  if (!ctx.result || !ctx.result.items) {
    return [];
  }

  return ctx.result.items.map((item) => ({
    slug: item.SK,
    description: item.description ?? null,
    created_at: item.created_at ?? 0,
    created_by: item.created_by ?? null,
  }));
}
