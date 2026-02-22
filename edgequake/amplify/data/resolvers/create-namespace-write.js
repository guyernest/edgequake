/**
 * AppSync JS pipeline resolver (step 2 of 2): createNamespace - Write
 *
 * Writes all namespace records atomically using TransactWriteItems:
 * 1. META record (PK=NS#{slug}, SK=META)
 * 2. CONFIG record (PK=NS#{slug}, SK=CONFIG)
 * 3. NAMESPACES listing entry (PK=NAMESPACES, SK={slug})
 *
 * Uses conditional put on META to ensure uniqueness (race condition guard).
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  const slug = ctx.stash.slug;

  return {
    operation: 'TransactWriteItems',
    transactItems: [
      {
        table: ctx.env.NAMESPACE_TABLE,
        operation: 'PutItem',
        key: util.dynamodb.toMapValues({
          PK: `NS#${slug}`,
          SK: 'META',
        }),
        attributeValues: util.dynamodb.toMapValues({
          data: ctx.stash.metaData,
        }),
        condition: {
          expression: 'attribute_not_exists(PK)',
        },
      },
      {
        table: ctx.env.NAMESPACE_TABLE,
        operation: 'PutItem',
        key: util.dynamodb.toMapValues({
          PK: `NS#${slug}`,
          SK: 'CONFIG',
        }),
        attributeValues: util.dynamodb.toMapValues({
          data: ctx.stash.configData,
        }),
      },
      {
        table: ctx.env.NAMESPACE_TABLE,
        operation: 'PutItem',
        key: util.dynamodb.toMapValues({
          PK: 'NAMESPACES',
          SK: slug,
        }),
        attributeValues: util.dynamodb.toMapValues({
          description: ctx.stash.description ?? '',
          created_at: ctx.stash.created_at,
        }),
      },
    ],
  };
}

export function response(ctx) {
  if (ctx.error) {
    // TransactWriteItems can fail on condition check (race condition)
    if (ctx.error.type === 'TransactionCanceledException') {
      return util.error('Namespace already exists', 'ConflictError');
    }
    return util.error(ctx.error.message, ctx.error.type);
  }

  // Return the namespace object (from previous pipeline step)
  return ctx.prev.result;
}
