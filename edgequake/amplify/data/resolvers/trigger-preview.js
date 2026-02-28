/**
 * AppSync JS pipeline resolver (step 1 of 2): triggerPreviewExtraction - Read/Validate
 *
 * Reads the PREVIEW_REQUEST record for a namespace to check if a preview
 * is already running (concurrent guard). If active, returns a ConflictError.
 * If idle/completed/failed/null, prepares a preview request for step 2 to write.
 *
 * Only blocks on 'processing' (not 'requested') because the CLI writes a
 * PREVIEW_RESULT record on completion but never updates PREVIEW_REQUEST.
 * A stale 'requested' status means the previous run either completed or was
 * never picked up -- both are safe to overwrite.
 *
 * Follows the same pattern as trigger-ingestion.js.
 */
import { util } from '@aws-appsync/utils';

const ACTIVE_STATUSES = ['processing'];

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

  // Check for active preview
  if (ctx.result) {
    const existing = JSON.parse(ctx.result.data);
    if (ACTIVE_STATUSES.includes(existing.status)) {
      return util.error(
        `A preview extraction is already ${existing.status}. Wait for completion before starting a new one.`,
        'ConflictError'
      );
    }
  }

  const slug = ctx.args.namespace;

  // Prepare preview request
  const previewRequest = {
    namespace: slug,
    requested_at: Math.floor(util.time.nowEpochSeconds()),
    status: 'requested',
    documents_completed: 0,
    documents_total: 3,
  };

  // Generate CLI command for operators
  const cliCommand = `edgequake-batch --namespace ${slug} --data <path-to-documents> preview`;

  // Pass to write step via stash
  ctx.stash.previewRequest = JSON.stringify(previewRequest);
  ctx.stash.slug = slug;
  ctx.stash.cliCommand = cliCommand;

  return {
    status: 'requested',
    documentsCompleted: 0,
    documentsTotal: 3,
    cliCommand: cliCommand,
  };
}
