/**
 * @module namespace-resolve
 * @description Resolves the namespace slug from a workspace context.
 *
 * Wave-0 gate — CASE A: Plan 01 Task 3 added `namespace_slug: String` to
 * WorkspaceResponse (workspaces_types.rs), set as `workspace.slug.clone()`.
 * The workspace→namespace mapping is authoritative: workspace.slug IS the
 * namespace slug (confirmed by tracing NamespaceSlug::parse call sites).
 *
 * Usage: pass to all /namespaces/{slug}/* API routes.
 */

import type { Workspace } from "@/types";

// ---------------------------------------------------------------------------
// Typed error
// ---------------------------------------------------------------------------

/**
 * Thrown when a namespace slug cannot be resolved from a workspace.
 * Plans 04/05/06 catch this to show an actionable UI error rather than a
 * generic "500 something went wrong."
 */
export class NamespaceSlugError extends Error {
  constructor(workspaceId: string) {
    super(
      `Cannot resolve namespace slug for workspace "${workspaceId}". ` +
        `The workspace is missing the namespace_slug field. ` +
        `Ensure the server is running Plan 01+ which populates WorkspaceResponse.namespace_slug.`,
    );
    this.name = "NamespaceSlugError";
    // Maintains correct instanceof checks in compiled-down ES5
    Object.setPrototypeOf(this, NamespaceSlugError.prototype);
  }
}

// ---------------------------------------------------------------------------
// Resolver
// ---------------------------------------------------------------------------

/**
 * Resolve the namespace slug from a workspace object.
 *
 * CASE A (active): reads `workspace.namespace_slug` which the server populates
 * as `workspace.slug` on every WorkspaceResponse (Plan 01 Task 3). This is the
 * same slug value that NamespaceSlug::parse accepts on the /namespaces/{ns}/*
 * routes.
 *
 * @param workspace - The workspace from WorkspaceResponse.
 * @returns The namespace slug string.
 * @throws {NamespaceSlugError} When no resolvable namespace slug is found.
 */
export function resolveNamespaceSlug(workspace: Workspace): string {
  if (workspace.namespace_slug && workspace.namespace_slug.length > 0) {
    return workspace.namespace_slug;
  }
  throw new NamespaceSlugError(workspace.id);
}
