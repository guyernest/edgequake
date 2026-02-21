/**
 * AppSync JS pipeline resolver (step 1 of 2): createNamespace - Read/Validate
 *
 * Checks if namespace already exists by reading PK=NS#{slug}, SK=META.
 * If it exists, returns a ConflictError.
 * If not, prepares META and default CONFIG data for step 2 to write.
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {
    operation: 'GetItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.args.slug}`,
      SK: 'META',
    }),
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }

  // If namespace already exists, conflict
  if (ctx.result) {
    return util.error('Namespace already exists', 'ConflictError');
  }

  const now = util.time.nowEpochMilliSeconds();

  // Prepare META record
  const meta = {
    slug: ctx.args.slug,
    description: ctx.args.description ?? '',
    created_at: now,
    created_by: ctx.identity?.username ?? ctx.identity?.sub ?? 'unknown',
  };

  // Prepare default CONFIG record
  const defaultConfig = {
    llm_provider: 'openai',
    llm_model: 'gpt-4o',
    embedding_provider: 'openai',
    embedding_model: 'text-embedding-3-large',
    embedding_dimension: 1024,
    chunking_strategy: 'fixed',
    chunk_size: 1000,
    chunk_overlap: 200,
    extraction_prompt: null,
    entity_types: [],
    relation_types: [],
    updated_at: now,
  };

  // Pass data to write step via stash
  ctx.stash.slug = ctx.args.slug;
  ctx.stash.metaData = JSON.stringify(meta);
  ctx.stash.configData = JSON.stringify(defaultConfig);
  ctx.stash.description = meta.description;
  ctx.stash.created_at = now;

  return meta;
}
