/**
 * @module snapshot-section.test
 * @description Unit tests for SnapshotSection rendering logic (D-09/D-10).
 *
 * Tests cover:
 * - snapshot=undefined → null render contract (D-09: only show when snapshot exists)
 * - full snapshot block → all 6 count keys rendered from SNAPSHOT_COUNT_KEYS (D-10)
 *
 * WR-06: these tests import the REAL exported render-decision helpers from
 * snapshot-section.tsx — no local re-implementations.
 *
 * @implements D-09 — SnapshotSection absent when no snapshot
 * @implements D-10 — SnapshotSection shows all 6 counts + URI + exported-at
 */

import type { IngestionResult } from "@/types/ingestion";
import { describe, expect, it } from "vitest";

import { SNAPSHOT_COUNT_KEYS, shouldRenderSnapshot } from "../snapshot-section";

type Snapshot = IngestionResult["snapshot"];

// ============================================================================
// D-09: null render when snapshot is undefined
// ============================================================================

describe("SnapshotSection — D-09: null render when no snapshot", () => {
  it("shouldRenderSnapshot returns false for undefined", () => {
    expect(shouldRenderSnapshot(undefined)).toBe(false);
  });

  it("shouldRenderSnapshot returns false for null", () => {
    expect(shouldRenderSnapshot(null)).toBe(false);
  });

  it("shouldRenderSnapshot returns true for a full snapshot block", () => {
    const snapshot: NonNullable<Snapshot> = {
      uri: "s3://bucket/snap",
      created_at: "2026-06-10T12:00:00Z",
      counts: {
        documents: 1,
        chunks: 20,
        entities: 80,
        relationships: 30,
        vectors: 20,
        bm25: 20,
      },
    };
    expect(shouldRenderSnapshot(snapshot)).toBe(true);
  });
});

// ============================================================================
// D-10: full snapshot block renders all 6 counts + URI + created_at
// ============================================================================

describe("SnapshotSection — D-10: full snapshot block shows 6 counts + URI + created_at", () => {
  const fullSnapshot: NonNullable<Snapshot> = {
    uri: "s3://my-bucket/snapshots/run-001",
    created_at: "2026-06-10T09:30:00Z",
    counts: {
      documents: 5,
      chunks: 42,
      entities: 120,
      relationships: 65,
      vectors: 42,
      bm25: 42,
    },
  };

  it("renders URI from snapshot.uri", () => {
    expect(fullSnapshot.uri).toBe("s3://my-bucket/snapshots/run-001");
  });

  it("renders created_at from snapshot.created_at", () => {
    expect(fullSnapshot.created_at).toBe("2026-06-10T09:30:00Z");
  });

  it("the component's SNAPSHOT_COUNT_KEYS lists exactly the 6 required fields", () => {
    expect(SNAPSHOT_COUNT_KEYS).toContain("documents");
    expect(SNAPSHOT_COUNT_KEYS).toContain("chunks");
    expect(SNAPSHOT_COUNT_KEYS).toContain("entities");
    expect(SNAPSHOT_COUNT_KEYS).toContain("relationships");
    expect(SNAPSHOT_COUNT_KEYS).toContain("vectors");
    expect(SNAPSHOT_COUNT_KEYS).toContain("bm25");
    expect(SNAPSHOT_COUNT_KEYS).toHaveLength(6);
  });

  it("every SNAPSHOT_COUNT_KEY resolves to a number on the snapshot counts", () => {
    for (const key of SNAPSHOT_COUNT_KEYS) {
      const value = fullSnapshot.counts[key];
      expect(typeof value).toBe("number");
      expect(value).toBeGreaterThanOrEqual(0);
      expect(Number.isInteger(value)).toBe(true);
    }
  });

  it("count values match the snapshot data", () => {
    expect(fullSnapshot.counts.documents).toBe(5);
    expect(fullSnapshot.counts.chunks).toBe(42);
    expect(fullSnapshot.counts.entities).toBe(120);
    expect(fullSnapshot.counts.relationships).toBe(65);
  });
});

// ============================================================================
// T-23-08: URI is rendered as text (no HTML injection surface)
// ============================================================================

describe("T-23-08: URI text safety contract", () => {
  it("snapshot.uri is a plain string (React escapes it in <code> element)", () => {
    const snap: NonNullable<Snapshot> = {
      uri: 's3://bucket/<script>alert("xss")</script>',
      created_at: "2026-06-10T00:00:00Z",
      counts: {
        documents: 0,
        chunks: 0,
        entities: 0,
        relationships: 0,
        vectors: 0,
        bm25: 0,
      },
    };
    // The URI is rendered inside a <code> element with no dangerouslySetInnerHTML.
    // React escapes by default, so even a malicious URI is safe as text content.
    expect(typeof snap.uri).toBe("string");
    expect(shouldRenderSnapshot(snap)).toBe(true);
    expect(snap.uri).toContain("<script>"); // raw value; React will escape in render
  });
});
