/**
 * AppSync JS pipeline resolver (step 2 of 3): triggerIngestion - Read/Validate
 *
 * Gets the LATEST_RUN record for a namespace to check if a pipeline run
 * is currently active (concurrent run guard). If active, returns a ConflictError.
 * If idle/completed/failed/null, prepares a run request for step 3 to write.
 *
 * Step 1 (schema gate) has already verified an approved schema exists.
 */
import { util } from '@aws-appsync/utils';

const ACTIVE_STATUSES = ['running', 'preparing', 'extracting', 'embedding', 'storing', 'requested'];

export function request(ctx) {
  return {
    operation: 'GetItem',
    key: util.dynamodb.toMapValues({
      PK: `NS#${ctx.args.slug}`,
      SK: 'LATEST_RUN',
    }),
  };
}

export function response(ctx) {
  if (ctx.error) {
    return util.error(ctx.error.message, ctx.error.type);
  }

  // Check for active run
  if (ctx.result) {
    const latestRun = JSON.parse(ctx.result.data);
    if (ACTIVE_STATUSES.includes(latestRun.status)) {
      return util.error(
        'Pipeline is currently running. Wait for completion before starting a new run.',
        'ConflictError'
      );
    }
  }

  const slug = ctx.args.slug;
  const batchSize = ctx.args.batch_size;
  const offset = ctx.args.offset;
  const dataPath = ctx.args.data_path;

  // Prepare run request
  const runRequest = {
    namespace: slug,
    batch_size: batchSize,
    offset: offset,
    data_path: dataPath,
    requested_at: util.time.nowEpochMilliSeconds(),
    status: 'requested',
  };

  // Generate CLI command for operators
  const cliCommand = `edgequake-batch --namespace ${slug} --data ${dataPath} --limit ${batchSize} --offset ${offset} run`;

  // Pass to write step via stash
  ctx.stash.runRequest = JSON.stringify(runRequest);
  ctx.stash.slug = slug;
  ctx.stash.cliCommand = cliCommand;

  return { ...runRequest, cli_command: cliCommand };
}
