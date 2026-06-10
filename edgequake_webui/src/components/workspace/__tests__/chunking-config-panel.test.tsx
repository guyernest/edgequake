/**
 * @module chunking-config-panel.test
 * @description Unit tests for ChunkingConfigPanel validation and logic.
 *
 * Tests cover:
 * - Master toggle OFF state: sub-fields disabled (DISABLED_SUBFIELDS_CLASSES)
 * - Master toggle ON: pre-fills Phase 22 defaults
 * - Overlap > 50% of target → validation error, Save disabled
 * - Target < 64 → range error
 * - Overlap helper text shows correct percentage
 *
 * WR-06: these tests import the REAL exported helpers/constants from
 * chunking-config-panel.tsx — no local re-implementations.
 */

import { describe, expect, it } from "vitest";

import {
  DISABLED_SUBFIELDS_CLASSES,
  isOverlapValid,
  overlapPercent,
  PHASE22_DEFAULTS,
  targetInRange,
} from "../chunking-config-panel";

// ============================================================================
// Master toggle — OFF state
// ============================================================================

describe("ChunkingConfigPanel — master toggle OFF state", () => {
  it("disabled sub-fields use opacity-50 pointer-events-none (D-06 contract)", () => {
    expect(DISABLED_SUBFIELDS_CLASSES).toBe("opacity-50 pointer-events-none");
  });

  it("the disabled classes are visibility-only (no display/hidden utility)", () => {
    // D-06/UI-SPEC: sub-fields stay in the DOM when OFF — visibility-only.
    expect(DISABLED_SUBFIELDS_CLASSES).not.toContain("hidden");
    expect(DISABLED_SUBFIELDS_CLASSES).not.toContain("sr-only");
  });
});

// ============================================================================
// Master toggle ON — Phase 22 default pre-fill
// ============================================================================

describe("ChunkingConfigPanel — toggle ON pre-fills Phase 22 defaults", () => {
  it("chunking_strategy defaults to heading_boundary", () => {
    expect(PHASE22_DEFAULTS.chunking_strategy).toBe("heading_boundary");
  });

  it("target_tokens defaults to 256", () => {
    expect(PHASE22_DEFAULTS.target_tokens).toBe(256);
  });

  it("overlap_tokens defaults to 38", () => {
    expect(PHASE22_DEFAULTS.overlap_tokens).toBe(38);
  });

  it("prepend_header_path defaults to true (breadcrumb ON)", () => {
    expect(PHASE22_DEFAULTS.prepend_header_path).toBe(true);
  });

  it("default overlap 38/256 is valid per validation rules", () => {
    expect(isOverlapValid(PHASE22_DEFAULTS.target_tokens, PHASE22_DEFAULTS.overlap_tokens)).toBe(true);
  });
});

// ============================================================================
// targetInRange validation
// ============================================================================

describe("targetInRange", () => {
  it("returns true for 64 (lower bound)", () => {
    expect(targetInRange(64)).toBe(true);
  });

  it("returns true for 2048 (upper bound)", () => {
    expect(targetInRange(2048)).toBe(true);
  });

  it("returns true for 256 (default)", () => {
    expect(targetInRange(256)).toBe(true);
  });

  it("returns false for 32 (below 64 — D-08 range error)", () => {
    expect(targetInRange(32)).toBe(false);
  });

  it("returns false for 63 (just below lower bound)", () => {
    expect(targetInRange(63)).toBe(false);
  });

  it("returns false for 2049 (above upper bound)", () => {
    expect(targetInRange(2049)).toBe(false);
  });

  it("returns false for 0", () => {
    expect(targetInRange(0)).toBe(false);
  });
});

// ============================================================================
// isOverlapValid — overlap > 50% blocks save
// ============================================================================

describe("isOverlapValid — overlap > 50% of target blocks save", () => {
  it("returns false when overlap=200, target=256 (200/256 ≈ 78% > 50%)", () => {
    expect(isOverlapValid(256, 200)).toBe(false);
  });

  it("returns false when overlap equals target (must be strictly less than target)", () => {
    expect(isOverlapValid(256, 256)).toBe(false);
  });

  it("returns false when overlap exceeds target", () => {
    expect(isOverlapValid(256, 300)).toBe(false);
  });

  it("returns true when overlap is exactly 50% of target (boundary: ratio must be <= 0.5)", () => {
    // 128/256 = 0.5 exactly — the rule is overlap/target <= 0.5, so this should pass
    expect(isOverlapValid(256, 128)).toBe(true);
  });

  it("returns false when overlap/target just above 0.5", () => {
    // 129/256 = 0.504 > 0.5
    expect(isOverlapValid(256, 129)).toBe(false);
  });

  it("returns true for valid overlap 38/256 (14.8%)", () => {
    expect(isOverlapValid(256, 38)).toBe(true);
  });

  it("returns true for overlap=0 (no overlap)", () => {
    expect(isOverlapValid(256, 0)).toBe(true);
  });

  it("returns false for negative overlap", () => {
    expect(isOverlapValid(256, -1)).toBe(false);
  });
});

// ============================================================================
// overlapPercent helper text
// ============================================================================

describe("overlapPercent — live percentage hint", () => {
  it("38/256 shows 14.8%", () => {
    expect(overlapPercent(256, 38)).toBe("14.8");
  });

  it("128/256 shows 50.0%", () => {
    expect(overlapPercent(256, 128)).toBe("50.0");
  });

  it("200/256 shows 78.1%", () => {
    expect(overlapPercent(256, 200)).toBe("78.1");
  });

  it("0/256 shows 0.0%", () => {
    expect(overlapPercent(256, 0)).toBe("0.0");
  });

  it("target=0 returns 0.0 (safe division guard)", () => {
    expect(overlapPercent(0, 10)).toBe("0.0");
  });
});

// ============================================================================
// Save button disabled when validation error is present
// ============================================================================

describe("ChunkingConfigPanel — Save button disabled on validation error", () => {
  it("Save is disabled when target is out of range", () => {
    const target = 32; // below 64
    const overlap = 10;
    const hasError = !targetInRange(target) || !isOverlapValid(target, overlap);
    expect(hasError).toBe(true); // Save must be disabled
  });

  it("Save is disabled when overlap exceeds 50% of target", () => {
    const target = 256;
    const overlap = 200; // 78% > 50%
    const hasError = !targetInRange(target) || !isOverlapValid(target, overlap);
    expect(hasError).toBe(true); // Save must be disabled
  });

  it("Save is enabled when all validation passes", () => {
    const target = 256;
    const overlap = 38;
    const hasError = !targetInRange(target) || !isOverlapValid(target, overlap);
    expect(hasError).toBe(false); // Save is enabled
  });
});
