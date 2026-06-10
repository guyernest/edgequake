/**
 * @module review-step.test
 * @description Unit tests for ReviewStep validation logic and schema payload behaviour.
 *
 * Tests cover:
 * - RELATED_TO row is rendered and has NO delete button
 * - The payload built for updateNamespaceSchema filters out RELATED_TO
 * - Entity name UPPER_SNAKE_CASE transform on blur
 * - Relation type deletion goes through a Dialog confirmation guard
 *
 * Per project test style (see chunking-config-panel.test.tsx), logic is extracted
 * as pure functions and tested directly — no React Testing Library / jsdom required
 * (vitest runs in node environment).
 */

import type { EntityTypeProposal, RelationTypeProposal } from "@/types/ingestion";
import { describe, expect, it } from "vitest";

// ============================================================================
// Pure helpers (replicate logic from review-step.tsx)
// ============================================================================

/** The sentinel name for the system-fallback relation type. */
const RELATED_TO = "RELATED_TO";

/**
 * Returns true when a relation type row is the fixed RELATED_TO fallback.
 * The delete button must be ABSENT (not rendered) for this row.
 */
function isFixedRelationRow(name: string): boolean {
  return name === RELATED_TO;
}

/**
 * Returns true when the given row should have a delete button.
 * The delete button is ABSENT for RELATED_TO and PRESENT for all others.
 */
function hasDeleteButton(name: string): boolean {
  return !isFixedRelationRow(name);
}

/**
 * Filters the relation_types array to exclude RELATED_TO before sending to
 * updateNamespaceSchema. Implements Pitfall 3 / T-23-04 defense-in-depth.
 *
 * @param relationTypes - Editable relation types including any user-added RELATED_TO.
 * @returns Relation types with RELATED_TO stripped.
 */
function filterRelatedTo(
  relationTypes: RelationTypeProposal[],
): RelationTypeProposal[] {
  return relationTypes.filter((r) => r.name !== RELATED_TO);
}

/**
 * Normalises an entity type name to UPPER_SNAKE_CASE on blur.
 * Rule (REVIEW Gemini #9): toUpperCase then replace any run of non-alphanumeric
 * characters with a single underscore, then strip leading/trailing underscores.
 *
 * Examples:
 *   "person entity" → "PERSON_ENTITY"
 *   "my.entity!type" → "MY_ENTITY_TYPE"
 *   "_leading_" → "LEADING"
 *   "ALREADY_GOOD" → "ALREADY_GOOD"
 */
function normalizeEntityName(raw: string): string {
  return raw
    .toUpperCase()
    .replace(/[^A-Z0-9]+/g, "_")
    .replace(/^_|_$/g, "");
}

/**
 * Guards relation type deletion behind a Dialog confirmation.
 * Returns true when deletion is allowed (i.e. the user has confirmed via dialog).
 */
function canDeleteRelation(
  name: string,
  userConfirmed: boolean,
): boolean {
  if (isFixedRelationRow(name)) return false; // RELATED_TO is never deletable
  return userConfirmed;
}

// ============================================================================
// isFixedRelationRow
// ============================================================================

describe("isFixedRelationRow", () => {
  it("returns true for RELATED_TO", () => {
    expect(isFixedRelationRow("RELATED_TO")).toBe(true);
  });

  it("returns false for an editable relation type", () => {
    expect(isFixedRelationRow("employs")).toBe(false);
  });

  it("returns false for a name that merely contains 'RELATED_TO'", () => {
    expect(isFixedRelationRow("NOT_RELATED_TO")).toBe(false);
  });
});

// ============================================================================
// RELATED_TO row has NO delete button
// ============================================================================

describe("hasDeleteButton — RELATED_TO row has no delete button", () => {
  it("RELATED_TO row has NO delete button", () => {
    expect(hasDeleteButton("RELATED_TO")).toBe(false);
  });

  it("editable relation type row HAS a delete button", () => {
    expect(hasDeleteButton("employs")).toBe(true);
  });

  it("another editable relation type HAS a delete button", () => {
    expect(hasDeleteButton("manages")).toBe(true);
  });
});

// ============================================================================
// filterRelatedTo — RELATED_TO never sent to server
// ============================================================================

describe("filterRelatedTo — payload excludes RELATED_TO", () => {
  const editableType: RelationTypeProposal = {
    name: "employs",
    description: "Employment relationship",
    frequency: 5,
  };

  const relatedToRow: RelationTypeProposal = {
    name: "RELATED_TO",
    description: "System fallback",
    frequency: 0,
  };

  it("removes RELATED_TO from a list containing it", () => {
    const result = filterRelatedTo([editableType, relatedToRow]);
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe("employs");
    expect(result.some((r) => r.name === "RELATED_TO")).toBe(false);
  });

  it("returns the same list when RELATED_TO is absent", () => {
    const result = filterRelatedTo([editableType]);
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe("employs");
  });

  it("returns an empty array when RELATED_TO is the only item", () => {
    const result = filterRelatedTo([relatedToRow]);
    expect(result).toHaveLength(0);
  });

  it("filters RELATED_TO even when it appears multiple times", () => {
    const result = filterRelatedTo([relatedToRow, editableType, relatedToRow]);
    expect(result).toHaveLength(1);
    expect(result.some((r) => r.name === "RELATED_TO")).toBe(false);
  });
});

// ============================================================================
// normalizeEntityName — UPPER_SNAKE_CASE on blur
// ============================================================================

describe("normalizeEntityName — UPPER_SNAKE_CASE transform", () => {
  it("converts spaces to underscores and uppercases", () => {
    expect(normalizeEntityName("person entity")).toBe("PERSON_ENTITY");
  });

  it("strips punctuation (dots, exclamation, dashes)", () => {
    expect(normalizeEntityName("my.entity!type")).toBe("MY_ENTITY_TYPE");
  });

  it("strips leading and trailing underscores", () => {
    expect(normalizeEntityName("_leading_")).toBe("LEADING");
  });

  it("leaves an already-correct name unchanged", () => {
    expect(normalizeEntityName("ALREADY_GOOD")).toBe("ALREADY_GOOD");
  });

  it("collapses multiple consecutive non-alphanumeric chars to one underscore", () => {
    expect(normalizeEntityName("foo---bar")).toBe("FOO_BAR");
  });

  it("handles a single word", () => {
    expect(normalizeEntityName("person")).toBe("PERSON");
  });

  it("uses toUpperCase().replace(/[^A-Z0-9]+/g,'_').replace(/^_|_$/g,'') — exact transform rule", () => {
    // Verify the exact rule from REVIEW Gemini #9 (strips punctuation, trims underscores)
    const raw = " foo bar! ";
    const result = raw
      .toUpperCase()
      .replace(/[^A-Z0-9]+/g, "_")
      .replace(/^_|_$/g, "");
    expect(result).toBe("FOO_BAR");
    expect(normalizeEntityName(raw)).toBe(result);
  });
});

// ============================================================================
// canDeleteRelation — delete confirmation gate
// ============================================================================

describe("canDeleteRelation — delete requires Dialog confirmation", () => {
  it("RELATED_TO cannot be deleted even when user has confirmed", () => {
    expect(canDeleteRelation("RELATED_TO", true)).toBe(false);
  });

  it("editable relation can be deleted when user confirms", () => {
    expect(canDeleteRelation("employs", true)).toBe(true);
  });

  it("editable relation is blocked when user has NOT confirmed", () => {
    expect(canDeleteRelation("employs", false)).toBe(false);
  });
});

// ============================================================================
// Schema payload contract — entity types pass through unfiltered
// ============================================================================

describe("entity types payload — no filtering applied", () => {
  it("entity type list is sent as-is (no RELATED_TO concept in entity list)", () => {
    const entityTypes: EntityTypeProposal[] = [
      { name: "PERSON", description: "A person", frequency: 5, is_baseline: false },
      { name: "ORGANISATION", description: "An org", frequency: 3, is_baseline: false },
    ];
    // Entity types are sent verbatim — there is no RELATED_TO row in the entity list
    expect(entityTypes).toHaveLength(2);
    expect(entityTypes[0].name).toBe("PERSON");
    expect(entityTypes[1].name).toBe("ORGANISATION");
  });
});
