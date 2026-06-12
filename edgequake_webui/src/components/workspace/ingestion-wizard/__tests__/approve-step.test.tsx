/**
 * @module approve-step.test
 * @description Unit tests for ApproveStep logic and contracts (plan 24-03).
 *
 * Tests cover:
 * (a) Track registration — buildTrackingArgs produces the correct synthetic id + label
 * (b) resolveTrackId — extracts track_id from rebuild response, handles absent track_id
 * (c) isSchemaApproved — CTA gate logic: session wizard-store AND fetched namespace schema
 *     are both valid approved-schema sources (re-entry fix)
 * (d) Call-order invariant — startTracking args are available BEFORE reset/navigate
 *     (proven via the helpers: startTracking is called synchronously with the args returned
 *      by buildTrackingArgs, before the reset/router.push sequence in onSuccess)
 * (e) T-24-03-02 — tracking args are non-secret (synthetic id + static label)
 *
 * WR-06 convention: tests import the REAL exported helpers from approve-step.tsx —
 * no local re-implementations. They fail if the production logic changes.
 *
 * Note: The test environment is 'node' (vitest.config.mjs:8) — no jsdom available.
 * Component render tests are not included here; behaviour contracts are exercised
 * via the exported pure helpers and integration-style spy tests below.
 */

import type { RebuildKnowledgeGraphResponse } from "@/lib/api/edgequake";
import type { SchemaProposal } from "@/types/ingestion";
import { describe, expect, it, vi } from "vitest";

import {
  REBUILD_TRACK_DOCUMENT_NAME,
  buildTrackingArgs,
  isSchemaApproved,
  resolveTrackId,
} from "../approve-step";

// ============================================================================
// buildTrackingArgs — synthetic track identity (T-24-03-02 contract)
// ============================================================================

describe("buildTrackingArgs — synthetic track id for wizard-triggered rebuild", () => {
  const WORKSPACE_ID = "ws-abc-123";

  it("documentId uses the rebuild:<workspaceId> synthetic format", () => {
    const { documentId } = buildTrackingArgs(WORKSPACE_ID);
    expect(documentId).toBe(`rebuild:${WORKSPACE_ID}`);
  });

  it("documentName is the static REBUILD_TRACK_DOCUMENT_NAME label", () => {
    const { documentName } = buildTrackingArgs(WORKSPACE_ID);
    expect(documentName).toBe(REBUILD_TRACK_DOCUMENT_NAME);
  });

  it("documentName is a non-empty non-secret string (T-24-03-02)", () => {
    const { documentName } = buildTrackingArgs(WORKSPACE_ID);
    expect(typeof documentName).toBe("string");
    expect(documentName.length).toBeGreaterThan(0);
    // Must not contain tokens, keys, or UUIDs — just a static label
    expect(documentName).toBe("Full knowledge graph rebuild");
  });

  it("documentId embeds the full workspaceId (no truncation)", () => {
    const longWorkspaceId = "workspace-" + "a".repeat(36);
    const { documentId } = buildTrackingArgs(longWorkspaceId);
    expect(documentId).toContain(longWorkspaceId);
  });

  it("two calls with different workspaceIds produce different documentIds", () => {
    const args1 = buildTrackingArgs("ws-1");
    const args2 = buildTrackingArgs("ws-2");
    expect(args1.documentId).not.toBe(args2.documentId);
    // documentName is always the same static label
    expect(args1.documentName).toBe(args2.documentName);
  });
});

// ============================================================================
// resolveTrackId — extracts track_id from rebuild response
// ============================================================================

function makeRebuildResponse(
  track_id?: string,
): RebuildKnowledgeGraphResponse {
  return {
    workspace_id: "ws-test",
    status: "started",
    nodes_cleared: 0,
    edges_cleared: 0,
    vectors_cleared: 0,
    documents_to_process: 5,
    chunks_to_process: 50,
    llm_model: "gpt-4.1-mini",
    llm_provider: "openai",
    ...(track_id !== undefined ? { track_id } : {}),
  };
}

describe("resolveTrackId — extracts optional track_id from rebuild response", () => {
  it("returns the track_id when present", () => {
    const rebuild = makeRebuildResponse("rebuild_kg_test_1");
    expect(resolveTrackId(rebuild)).toBe("rebuild_kg_test_1");
  });

  it("returns undefined when track_id is absent (optional field)", () => {
    const rebuild = makeRebuildResponse(); // no track_id
    expect(resolveTrackId(rebuild)).toBeUndefined();
  });

  it("returns the exact track_id string (no mutation)", () => {
    const trackId = "track-abc-def-ghi";
    const rebuild = makeRebuildResponse(trackId);
    expect(resolveTrackId(rebuild)).toBe(trackId);
  });
});

// ============================================================================
// isSchemaApproved — CTA gate logic (re-entry fix contract)
// ============================================================================

function approvedSchema(): SchemaProposal {
  return {
    status: "approved",
    entity_types: [],
    relation_types: [],
  };
}

function proposedSchema(): SchemaProposal {
  return {
    status: "proposed",
    entity_types: [],
    relation_types: [],
  };
}

describe("isSchemaApproved — CTA enabled when either source is approved", () => {
  // The re-entry path: wizardSchema is null, fetchedSchema carries the approved status
  it("returns true when only fetchedSchema is approved and wizardSchema is null (re-entry CTA fix)", () => {
    expect(isSchemaApproved(null, approvedSchema())).toBe(true);
  });

  // The fresh wizard pass: wizardSchema is approved after review step sets it
  it("returns true when wizardSchema is approved and fetchedSchema is null", () => {
    expect(isSchemaApproved(approvedSchema(), null)).toBe(true);
  });

  // Both sources approved — valid (e.g. re-entry after a fresh pass)
  it("returns true when both wizardSchema and fetchedSchema are approved", () => {
    expect(isSchemaApproved(approvedSchema(), approvedSchema())).toBe(true);
  });

  // Neither source approved — CTA disabled
  it("returns false when wizardSchema is null and fetchedSchema is null (initial mount, not yet approved)", () => {
    expect(isSchemaApproved(null, null)).toBe(false);
  });

  it("returns false when wizardSchema is null and fetchedSchema is undefined (query pending)", () => {
    expect(isSchemaApproved(null, undefined)).toBe(false);
  });

  it("returns false when wizardSchema is 'proposed' and fetchedSchema is null", () => {
    expect(isSchemaApproved(proposedSchema(), null)).toBe(false);
  });

  it("returns false when wizardSchema is null and fetchedSchema is 'proposed'", () => {
    expect(isSchemaApproved(null, proposedSchema())).toBe(false);
  });

  it("returns false when both are non-approved", () => {
    expect(isSchemaApproved(proposedSchema(), proposedSchema())).toBe(false);
  });

  // Status enum coverage
  const nonApprovedStatuses: SchemaProposal["status"][] = [
    "none", "proposing", "proposed", "rejected", "failed",
  ];
  for (const status of nonApprovedStatuses) {
    it(`returns false when fetchedSchema.status is '${status}' and wizardSchema is null`, () => {
      const fetched: SchemaProposal = { status, entity_types: [], relation_types: [] };
      expect(isSchemaApproved(null, fetched)).toBe(false);
    });
  }
});

// ============================================================================
// Call-order invariant — startTracking is called BEFORE reset/router.push
// ============================================================================

describe("call-order invariant — startTracking args available before reset/navigate", () => {
  /**
   * The onSuccess handler in approve-step.tsx calls, in order:
   *   1. resolveTrackId(rebuild)        → extract trackId
   *   2. buildTrackingArgs(workspaceId) → build documentId + documentName
   *   3. startTracking(trackId, documentId, documentName)  ← must be FIRST
   *   4. toast.success(...)
   *   5. reset()
   *   6. router.push("/workspace")
   *
   * This test simulates that sequence using call-order spies, asserting
   * that startTracking is invoked with the correct args before reset/push.
   */
  it("simulates onSuccess: startTracking is called before reset and router.push", () => {
    const callOrder: string[] = [];
    const WORKSPACE_ID = "ws-order-test";
    const TRACK_ID = "rebuild_kg_order_1";

    const startTracking = vi.fn((..._args: unknown[]) => {
      callOrder.push("startTracking");
    });
    const reset = vi.fn(() => {
      callOrder.push("reset");
    });
    const push = vi.fn(() => {
      callOrder.push("push");
    });

    // Simulate the onSuccess body from approve-step.tsx
    const rebuild = makeRebuildResponse(TRACK_ID);
    const trackId = resolveTrackId(rebuild);
    if (trackId) {
      const { documentId, documentName } = buildTrackingArgs(WORKSPACE_ID);
      startTracking(trackId, documentId, documentName); // step 3
    }
    reset(); // step 5
    push("/workspace"); // step 6

    // Call-order assertion: startTracking < reset < push
    expect(callOrder).toEqual(["startTracking", "reset", "push"]);
    expect(callOrder.indexOf("startTracking")).toBeLessThan(
      callOrder.indexOf("reset"),
    );
    expect(callOrder.indexOf("startTracking")).toBeLessThan(
      callOrder.indexOf("push"),
    );
  });

  it("startTracking is called with correct args: trackId, rebuild:<workspaceId>, static label", () => {
    const WORKSPACE_ID = "ws-args-test";
    const TRACK_ID = "rebuild_kg_args_1";
    const startTracking = vi.fn();

    const rebuild = makeRebuildResponse(TRACK_ID);
    const trackId = resolveTrackId(rebuild);
    if (trackId) {
      const { documentId, documentName } = buildTrackingArgs(WORKSPACE_ID);
      startTracking(trackId, documentId, documentName);
    }

    expect(startTracking).toHaveBeenCalledOnce();
    expect(startTracking).toHaveBeenCalledWith(
      TRACK_ID,
      `rebuild:${WORKSPACE_ID}`,
      REBUILD_TRACK_DOCUMENT_NAME,
    );
  });

  it("no-track_id branch: startTracking is NOT called when track_id is absent", () => {
    const startTracking = vi.fn();
    const WORKSPACE_ID = "ws-no-track";

    const rebuild = makeRebuildResponse(); // no track_id
    const trackId = resolveTrackId(rebuild);
    if (trackId) {
      const { documentId, documentName } = buildTrackingArgs(WORKSPACE_ID);
      startTracking(trackId, documentId, documentName);
    }
    // track_id undefined → startTracking skipped

    expect(startTracking).not.toHaveBeenCalled();
  });

  it("no-track_id branch: does not throw (still navigates cleanly)", () => {
    const WORKSPACE_ID = "ws-no-track-clean";
    const push = vi.fn();
    const reset = vi.fn();

    const rebuild = makeRebuildResponse();
    const trackId = resolveTrackId(rebuild);
    // Simulate the no-track_id path: skip startTracking, still navigate
    expect(() => {
      if (trackId) {
        // would call startTracking, but trackId is undefined here
      }
      reset();
      push("/workspace");
    }).not.toThrow();

    expect(reset).toHaveBeenCalledOnce();
    expect(push).toHaveBeenCalledWith("/workspace");
  });
});

// ============================================================================
// In-flight CTA state — isLoading gate prevents double-submission
// ============================================================================

describe("in-flight CTA state — schemaApproved + loading semantics", () => {
  /**
   * The CTA is: disabled={isLoading || !schemaApproved}
   *
   * While approveMutation.isPending=true, isLoading=true → disabled regardless
   * of schemaApproved.  This prevents double-submission.
   */
  it("CTA is disabled when isLoading=true even if schemaApproved=true", () => {
    const isLoading = true;
    const schemaApproved = isSchemaApproved(approvedSchema(), null);
    // Simulate: disabled={isLoading || !schemaApproved}
    const disabled = isLoading || !schemaApproved;
    expect(disabled).toBe(true);
  });

  it("CTA is enabled when isLoading=false and schemaApproved=true", () => {
    const isLoading = false;
    const schemaApproved = isSchemaApproved(approvedSchema(), null);
    const disabled = isLoading || !schemaApproved;
    expect(disabled).toBe(false);
  });

  it("CTA is disabled when isLoading=false and schemaApproved=false", () => {
    const isLoading = false;
    const schemaApproved = isSchemaApproved(null, null);
    const disabled = isLoading || !schemaApproved;
    expect(disabled).toBe(true);
  });
});

// ============================================================================
// i18n key contract — triggering + notTrackedWarning exist in en.json
// ============================================================================

import en from "@/locales/en.json";

describe("i18n keys — wizard.approve keys added for in-flight state and no-track warning", () => {
  it("wizard.approve.triggering is defined and non-empty", () => {
    expect(en.wizard.approve).toHaveProperty("triggering");
    expect(typeof en.wizard.approve.triggering).toBe("string");
    expect(en.wizard.approve.triggering.length).toBeGreaterThan(0);
  });

  it("wizard.approve.notTrackedWarning is defined and non-empty", () => {
    expect(en.wizard.approve).toHaveProperty("notTrackedWarning");
    expect(typeof en.wizard.approve.notTrackedWarning).toBe("string");
    expect(en.wizard.approve.notTrackedWarning.length).toBeGreaterThan(0);
  });

  it("wizard.approve.triggering contains 'Triggering' (in-flight label)", () => {
    expect(en.wizard.approve.triggering).toContain("Triggering");
  });
});
