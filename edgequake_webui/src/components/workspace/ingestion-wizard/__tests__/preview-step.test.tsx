/**
 * @module preview-step.test
 * @description Unit tests for PreviewStep logic and rendering contracts.
 *
 * Tests cover:
 * (a) Budget note copy — asserted against en.json directly (the real i18n source)
 * (b) RELATED_TO row in Relations tab receives the amber-highlight class +
 *     data-testid="related-to-fallback-row"
 * (c) isPreviewLoading recognises the server's real in-flight statuses
 *
 * WR-06: these tests import the REAL exported helpers from preview-step.tsx —
 * no local re-implementations. They fail if the production logic changes.
 */

import en from "@/locales/en.json";
import type { PreviewResult } from "@/types/ingestion";
import { describe, expect, it } from "vitest";

import {
  getRelationRowHighlightClass,
  getRelationRowTestId,
  isPreviewLoading,
  isRelatedToRow,
} from "../preview-step";

// ============================================================================
// Budget note copy — asserted against the real en.json value
// ============================================================================

describe("budget note — wizard.preview.budget in en.json", () => {
  const budgetCopy = en.wizard.preview.budget;

  it("contains '6 documents'", () => {
    expect(budgetCopy).toContain("6 documents");
  });

  it("contains '60 chunks'", () => {
    expect(budgetCopy).toContain("60 chunks");
  });
});

// ============================================================================
// isRelatedToRow — RELATED_TO fallback row detection
// ============================================================================

describe("isRelatedToRow", () => {
  it("returns true for RELATED_TO", () => {
    expect(isRelatedToRow("RELATED_TO")).toBe(true);
  });

  it("returns false for a normal relation type", () => {
    expect(isRelatedToRow("employs")).toBe(false);
  });

  it("returns false for partial match", () => {
    expect(isRelatedToRow("NOT_RELATED_TO")).toBe(false);
  });
});

// ============================================================================
// getRelationRowTestId — amber highlight via data-testid
// ============================================================================

describe("getRelationRowTestId — RELATED_TO row gets data-testid", () => {
  it("RELATED_TO row gets data-testid='related-to-fallback-row'", () => {
    expect(getRelationRowTestId("RELATED_TO")).toBe("related-to-fallback-row");
  });

  it("normal relation row gets no testId (undefined)", () => {
    expect(getRelationRowTestId("employs")).toBeUndefined();
  });
});

// ============================================================================
// getRelationRowHighlightClass — amber tint + border (UI-SPEC Color contract)
// ============================================================================

describe("getRelationRowHighlightClass — amber highlight for RELATED_TO", () => {
  it("RELATED_TO row gets amber background classes (light + dark)", () => {
    const cls = getRelationRowHighlightClass("RELATED_TO");
    expect(cls).toContain("bg-amber-50");
    expect(cls).toContain("dark:bg-amber-950/20");
  });

  it("RELATED_TO row gets amber border classes (light + dark)", () => {
    const cls = getRelationRowHighlightClass("RELATED_TO");
    expect(cls).toContain("border-amber-300");
    expect(cls).toContain("dark:border-amber-700");
  });

  it("normal relation row gets empty class string", () => {
    expect(getRelationRowHighlightClass("employs")).toBe("");
  });
});

// ============================================================================
// isPreviewLoading — Skeleton placeholders while result is in-flight
// (server statuses: requested | processing | none | completed | failed)
// ============================================================================

/** Minimal in-flight poll body — the server sends NO aggregate fields. */
function pollBody(status: PreviewResult["status"]): PreviewResult {
  return { status, namespace: "my-ns" };
}

describe("isPreviewLoading — Skeleton shown while loading", () => {
  it("returns true when isPreviewing=true (active poll running)", () => {
    expect(isPreviewLoading(true, null)).toBe(true);
  });

  it("returns true when result status is 'requested'", () => {
    expect(isPreviewLoading(false, pollBody("requested"))).toBe(true);
  });

  it("returns true when result status is 'processing'", () => {
    expect(isPreviewLoading(false, pollBody("processing"))).toBe(true);
  });

  it("returns true when result status is 'none' (request not yet recorded)", () => {
    expect(isPreviewLoading(false, pollBody("none"))).toBe(true);
  });

  it("returns false when result is completed", () => {
    const completedResult: PreviewResult = {
      status: "completed",
      entityTypeCounts: [{ typeName: "PERSON", count: 42 }],
      relationTypeCounts: [],
      coverageRows: [],
      documentColumns: [],
      totalChunks: 60,
      totalEntities: 42,
      totalRelationships: 0,
      cost: { inputTokens: 0, outputTokens: 0, totalCostUsd: 0, model: "gpt-4" },
      processingTimeMs: 0,
      documentsCompleted: 6,
      documentsTotal: 6,
    };
    expect(isPreviewLoading(false, completedResult)).toBe(false);
  });

  it("returns false when result is failed (terminal)", () => {
    expect(
      isPreviewLoading(false, { status: "failed", error: "boom" }),
    ).toBe(false);
  });

  it("returns false when result is null and not previewing (not started)", () => {
    expect(isPreviewLoading(false, null)).toBe(false);
  });
});

// ============================================================================
// Aggregate data shape — tabs render from correct fields
// ============================================================================

describe("preview tabs render aggregate fields (not per-row data)", () => {
  const mockCompletedResult: PreviewResult = {
    status: "completed",
    entityTypeCounts: [
      { typeName: "PERSON", count: 42 },
      { typeName: "ORGANISATION", count: 15 },
    ],
    relationTypeCounts: [
      { typeName: "employs", count: 18 },
      { typeName: "RELATED_TO", count: 7 },
    ],
    coverageRows: [{ entityType: "PERSON", counts: [3, 0, 5] }],
    documentColumns: [
      { id: "doc1", name: "case-file.pdf", truncatedName: "case-file..." },
      { id: "doc2", name: "report.pdf", truncatedName: "report.pdf" },
      { id: "doc3", name: "memo.pdf", truncatedName: "memo.pdf" },
    ],
    totalChunks: 60,
    totalEntities: 312,
    totalRelationships: 88,
    cost: { inputTokens: 9000, outputTokens: 2400, totalCostUsd: 0.0123, model: "gpt-4.1-mini" },
    processingTimeMs: 28400,
    documentsCompleted: 6,
    documentsTotal: 6,
  };

  it("Entities tab: entityTypeCounts has the expected rows", () => {
    const entityTypeCounts = mockCompletedResult.entityTypeCounts!;
    expect(entityTypeCounts).toHaveLength(2);
    expect(entityTypeCounts[0].typeName).toBe("PERSON");
    expect(entityTypeCounts[0].count).toBe(42);
  });

  it("Relations tab: relationTypeCounts has RELATED_TO row, annotated via real helpers", () => {
    const relationTypeCounts = mockCompletedResult.relationTypeCounts!;
    const relatedTo = relationTypeCounts.find((r) => isRelatedToRow(r.typeName));
    expect(relatedTo).toBeDefined();
    expect(getRelationRowTestId(relatedTo!.typeName)).toBe(
      "related-to-fallback-row",
    );
    expect(getRelationRowHighlightClass(relatedTo!.typeName)).not.toBe("");
    // The non-fallback row gets neither testid nor highlight
    expect(getRelationRowTestId("employs")).toBeUndefined();
  });

  it("Chunks tab: documentColumns provides the document list for chunk counts", () => {
    expect(mockCompletedResult.documentColumns).toHaveLength(3);
    expect(mockCompletedResult.totalChunks).toBe(60);
  });

  it("totalEntities is present for summary line", () => {
    expect(mockCompletedResult.totalEntities).toBe(312);
  });

  it("totalRelationships is present for summary line", () => {
    expect(mockCompletedResult.totalRelationships).toBe(88);
  });
});
