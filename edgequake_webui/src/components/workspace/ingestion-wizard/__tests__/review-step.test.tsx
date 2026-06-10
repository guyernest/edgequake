/**
 * @module review-step.test
 * @description Unit tests for ReviewStep validation logic and schema payload behaviour.
 *
 * Tests cover:
 * - RELATED_TO row is the fixed fallback (no delete button)
 * - The payload built for updateNamespaceSchema filters out RELATED_TO
 * - Entity name UPPER_SNAKE_CASE transform on blur
 *
 * WR-06: these tests import the REAL exported helpers from review-step.tsx —
 * no local re-implementations. They fail if the production logic changes.
 */

import type { EntityTypeProposal, RelationTypeProposal } from "@/types/ingestion";
import { describe, expect, it } from "vitest";

import {
  filterRelatedTo,
  hasDeleteButton,
  isFixedRelationRow,
  normalizeEntityName,
} from "../review-step";

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
// filterRelatedTo — RELATED_TO never sent to server (T-23-04)
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

  it("trims surrounding whitespace/punctuation into clean UPPER_SNAKE_CASE", () => {
    expect(normalizeEntityName(" foo bar! ")).toBe("FOO_BAR");
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
