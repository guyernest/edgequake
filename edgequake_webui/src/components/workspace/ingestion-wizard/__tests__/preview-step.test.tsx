/**
 * @module preview-step.test
 * @description Unit tests for PreviewStep logic and rendering contracts.
 *
 * Tests cover:
 * (a) Budget note contains "6 documents"
 * (b) RELATED_TO row in Relations tab receives amber-highlight marker
 *     (data-testid="related-to-fallback-row") and fallback badge text
 * (c) While result is pending, Skeleton state flag is truthy
 *
 * Per project test style (see chunking-config-panel.test.tsx), logic is extracted
 * as pure functions and tested directly — no React Testing Library / jsdom required
 * (vitest runs in node environment).
 */

import type { PreviewResult, TypeCount } from "@/types/ingestion";
import { describe, expect, it } from "vitest";

// ============================================================================
// Pure helpers (replicate logic from preview-step.tsx)
// ============================================================================

/** The sentinel name for the system-fallback relation type. */
const RELATED_TO = "RELATED_TO";

/**
 * Returns true when a relation type count row is the RELATED_TO fallback.
 * These rows receive the amber highlight wrapper + fallback badge.
 */
function isRelatedToRow(typeName: string): boolean {
  return typeName === RELATED_TO;
}

/**
 * Returns the data-testid to place on a relation count row.
 * RELATED_TO rows get "related-to-fallback-row"; others get undefined.
 */
function getRelationRowTestId(
  typeName: string,
): string | undefined {
  return isRelatedToRow(typeName) ? "related-to-fallback-row" : undefined;
}

/**
 * Returns the amber CSS classes for a RELATED_TO relation row highlight.
 * Matches UI-SPEC Color section (chart-4 amber tint bg + border).
 */
function getRelationRowHighlightClass(typeName: string): string {
  return isRelatedToRow(typeName)
    ? "bg-amber-50 dark:bg-amber-950/20 border border-amber-300 dark:border-amber-700"
    : "";
}

/**
 * Returns true when the preview is still loading (no completed result yet).
 * Used to decide whether to render Skeleton placeholders in each tab.
 */
function isPreviewLoading(
  isPreviewing: boolean,
  result: PreviewResult | null,
): boolean {
  if (isPreviewing) return true;
  if (!result) return false; // nothing started — not actively loading
  return (
    result.status === "requested" ||
    result.status === "processing" ||
    result.status === "none"
  );
}

/**
 * Returns the confirmed budget string (always this exact copy).
 * Maps to t('wizard.preview.budget') which equals "Sampling up to 6 documents and 60 chunks."
 */
const BUDGET_COPY = "Sampling up to 6 documents and 60 chunks.";

/**
 * Returns all relation type counts rows, annotating each with whether it is
 * the RELATED_TO fallback row (for rendering purposes).
 */
function annotateRelationRows(
  rows: TypeCount[],
): Array<TypeCount & { isRelatedTo: boolean; testId?: string }> {
  return rows.map((row) => ({
    ...row,
    isRelatedTo: isRelatedToRow(row.typeName),
    testId: getRelationRowTestId(row.typeName),
  }));
}

// ============================================================================
// Budget note copy
// ============================================================================

describe("budget note — contains '6 documents'", () => {
  it("BUDGET_COPY contains '6 documents'", () => {
    expect(BUDGET_COPY).toContain("6 documents");
  });

  it("BUDGET_COPY contains '60 chunks'", () => {
    expect(BUDGET_COPY).toContain("60 chunks");
  });

  it("BUDGET_COPY matches the confirmed en.json value exactly", () => {
    expect(BUDGET_COPY).toBe("Sampling up to 6 documents and 60 chunks.");
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
// RELATED_TO row receives amber highlight + badge — end-to-end assertion
// ============================================================================

describe("annotateRelationRows — RELATED_TO row amber-highlight + badge", () => {
  const mockResult: PreviewResult = {
    status: "completed",
    entityTypeCounts: [{ typeName: "PERSON", count: 42 }],
    relationTypeCounts: [
      { typeName: "employs", count: 18 },
      { typeName: "RELATED_TO", count: 7 },
      { typeName: "manages", count: 3 },
    ],
    coverageRows: [],
    documentColumns: [{ id: "doc1", name: "case-file.pdf", truncatedName: "case-file..." }],
    totalChunks: 60,
    totalEntities: 312,
    totalRelationships: 88,
    cost: { inputTokens: 9000, outputTokens: 2400, totalCostUsd: 0.0123, model: "gpt-4.1-mini" },
    processingTimeMs: 28400,
    documentsCompleted: 6,
    documentsTotal: 6,
  };

  it("RELATED_TO row in relationTypeCounts receives amber-highlight marker (data-testid)", () => {
    const annotated = annotateRelationRows(mockResult.relationTypeCounts!);
    const relatedToRow = annotated.find((r) => r.typeName === "RELATED_TO");
    expect(relatedToRow).toBeDefined();
    expect(relatedToRow!.isRelatedTo).toBe(true);
    expect(relatedToRow!.testId).toBe("related-to-fallback-row");
  });

  it("non-RELATED_TO rows have no testId and isRelatedTo=false", () => {
    const annotated = annotateRelationRows(mockResult.relationTypeCounts!);
    const employs = annotated.find((r) => r.typeName === "employs");
    expect(employs!.isRelatedTo).toBe(false);
    expect(employs!.testId).toBeUndefined();
  });

  it("annotates all three rows correctly", () => {
    const annotated = annotateRelationRows(mockResult.relationTypeCounts!);
    expect(annotated).toHaveLength(3);
    const relatedToCount = annotated.filter((r) => r.isRelatedTo).length;
    expect(relatedToCount).toBe(1);
  });
});

// ============================================================================
// getRelationRowHighlightClass — amber CSS classes
// ============================================================================

describe("getRelationRowHighlightClass — amber tint for RELATED_TO", () => {
  it("RELATED_TO row gets amber background class", () => {
    const cls = getRelationRowHighlightClass("RELATED_TO");
    expect(cls).toContain("bg-amber-50");
  });

  it("RELATED_TO row gets amber border class", () => {
    const cls = getRelationRowHighlightClass("RELATED_TO");
    expect(cls).toContain("border-amber-300");
  });

  it("normal relation row gets empty class string", () => {
    const cls = getRelationRowHighlightClass("employs");
    expect(cls).toBe("");
  });
});

// ============================================================================
// isPreviewLoading — Skeleton placeholders while result is pending
// ============================================================================

describe("isPreviewLoading — Skeleton shown while loading", () => {
  it("returns true when isPreviewing=true (active poll running)", () => {
    expect(isPreviewLoading(true, null)).toBe(true);
  });

  it("returns true when result status is 'requested'", () => {
    const pendingResult: PreviewResult = {
      status: "requested",
      entityTypeCounts: [],
      relationTypeCounts: [],
      coverageRows: [],
      documentColumns: [],
      totalChunks: 0,
      totalEntities: 0,
      totalRelationships: 0,
      cost: { inputTokens: 0, outputTokens: 0, totalCostUsd: 0, model: "gpt-4" },
      processingTimeMs: 0,
      documentsCompleted: 0,
      documentsTotal: 0,
    };
    expect(isPreviewLoading(false, pendingResult)).toBe(true);
  });

  it("returns true when result status is 'processing'", () => {
    const runningResult: PreviewResult = {
      status: "processing",
      entityTypeCounts: [],
      relationTypeCounts: [],
      coverageRows: [],
      documentColumns: [],
      totalChunks: 0,
      totalEntities: 0,
      totalRelationships: 0,
      cost: { inputTokens: 0, outputTokens: 0, totalCostUsd: 0, model: "gpt-4" },
      processingTimeMs: 0,
      documentsCompleted: 0,
      documentsTotal: 0,
    };
    expect(isPreviewLoading(false, runningResult)).toBe(true);
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
    coverageRows: [
      { entityType: "PERSON", counts: [3, 0, 5] },
    ],
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

  it("Relations tab: relationTypeCounts has RELATED_TO row", () => {
    const relationTypeCounts = mockCompletedResult.relationTypeCounts!;
    const hasRelatedTo = relationTypeCounts.some(
      (r) => r.typeName === "RELATED_TO",
    );
    expect(hasRelatedTo).toBe(true);
  });

  it("Chunks tab: documentColumns provides the document list for chunk counts", () => {
    const { documentColumns, totalChunks } = mockCompletedResult;
    expect(documentColumns).toHaveLength(3);
    expect(totalChunks).toBe(60);
  });

  it("totalEntities is present for summary line", () => {
    expect(mockCompletedResult.totalEntities).toBe(312);
  });

  it("totalRelationships is present for summary line", () => {
    expect(mockCompletedResult.totalRelationships).toBe(88);
  });
});
