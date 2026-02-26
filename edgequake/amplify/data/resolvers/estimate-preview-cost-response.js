/**
 * AppSync JS pipeline resolver (step 2 of 2): estimatePreviewCost - Response
 *
 * Shapes the cost estimate from the previous step into the GraphQL return type.
 * This is a NONE data source handler (no DynamoDB operation).
 */
import { util } from '@aws-appsync/utils';

export function request(ctx) {
  return {};
}

export function response(ctx) {
  return ctx.prev.result;
}
