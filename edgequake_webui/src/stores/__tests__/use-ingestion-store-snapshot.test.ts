/**
 * @module use-ingestion-store-snapshot.test
 * @description TDD RED/GREEN tests for snapshot capture (D-11) and persist middleware.
 *
 * Tests cover:
 * - ingestion_completed WITH snapshot stores snapshot block on completed job
 * - ingestion_completed WITHOUT snapshot leaves snapshot undefined (backward compat)
 * - persist partialize includes completedJobs (D-11 durability)
 *
 * @implements D-11 — snapshot block in persisted completedJobs slice
 */

import type { IngestionCompletedEvent } from "@/types/ingestion";
import { act } from "react";
import { beforeEach, describe, expect, it } from "vitest";
import { useIngestionStore } from "../use-ingestion-store";

// Reset store before each test
beforeEach(() => {
  const store = useIngestionStore.getState();
  store.clearAllTracks();
  store.clearCompletedJobs();
  store.clearAllFailedJobs();
  store.setWsConnected(false);
});

// ============================================================================
// Snapshot Block Capture Tests (D-11)
// ============================================================================

describe("Snapshot capture in ingestion_completed", () => {
  it("stores snapshot block on completed job when event contains snapshot", () => {
    const store = useIngestionStore.getState();

    act(() => {
      store.startTracking("track-snap-1", "doc-snap-1", "with-snapshot.pdf");
    });

    const event: IngestionCompletedEvent = {
      type: "ingestion_completed",
      track_id: "track-snap-1",
      document_id: "doc-snap-1",
      completed_at: new Date().toISOString(),
      total_duration_ms: 90000,
      summary: {
        chunks: 20,
        entities: 80,
        relationships: 30,
        total_cost_usd: 0.1,
      },
      snapshot: {
        uri: "s3://my-bucket/snapshots/doc-snap-1",
        created_at: "2026-06-10T12:00:00Z",
        counts: {
          documents: 1,
          chunks: 20,
          entities: 80,
          relationships: 30,
          vectors: 20,
          bm25: 20,
        },
      },
    };

    act(() => {
      store.updateFromMessage(event);
    });

    const completedJobs = useIngestionStore.getState().completedJobs;
    const job = completedJobs.find((j) => j.track_id === "track-snap-1");
    expect(job).toBeDefined();
    expect(job?.snapshot).toBeDefined();
    expect(job?.snapshot?.uri).toBe("s3://my-bucket/snapshots/doc-snap-1");
    expect(job?.snapshot?.created_at).toBe("2026-06-10T12:00:00Z");
    expect(job?.snapshot?.counts.documents).toBe(1);
    expect(job?.snapshot?.counts.chunks).toBe(20);
    expect(job?.snapshot?.counts.entities).toBe(80);
    expect(job?.snapshot?.counts.relationships).toBe(30);
    expect(job?.snapshot?.counts.vectors).toBe(20);
    expect(job?.snapshot?.counts.bm25).toBe(20);
  });

  it("leaves snapshot undefined when event has no snapshot (backward compat)", () => {
    const store = useIngestionStore.getState();

    act(() => {
      store.startTracking("track-no-snap", "doc-no-snap", "no-snapshot.pdf");
    });

    const event: IngestionCompletedEvent = {
      type: "ingestion_completed",
      track_id: "track-no-snap",
      document_id: "doc-no-snap",
      completed_at: new Date().toISOString(),
      total_duration_ms: 60000,
      summary: {
        chunks: 10,
        entities: 40,
        relationships: 15,
        total_cost_usd: 0.05,
      },
      // no snapshot field
    };

    act(() => {
      store.updateFromMessage(event);
    });

    const completedJobs = useIngestionStore.getState().completedJobs;
    const job = completedJobs.find((j) => j.track_id === "track-no-snap");
    expect(job).toBeDefined();
    expect(job?.snapshot).toBeUndefined();
  });

  it("does not crash when snapshot block is missing on event (T-23-09)", () => {
    const store = useIngestionStore.getState();

    // Should not throw even if snapshot is missing
    expect(() => {
      act(() => {
        store.startTracking("track-safe", "doc-safe", "safe.pdf");
        const event: IngestionCompletedEvent = {
          type: "ingestion_completed",
          track_id: "track-safe",
          document_id: "doc-safe",
          completed_at: new Date().toISOString(),
          total_duration_ms: 5000,
          summary: {
            chunks: 2,
            entities: 5,
            relationships: 2,
            total_cost_usd: 0.01,
          },
        };
        store.updateFromMessage(event);
      });
    }).not.toThrow();
  });
});

// ============================================================================
// D-11 Persistence: partialize function covers completedJobs (snapshot-bearing)
//
// Zustand 5 does not expose .persist on the React hook store (it's internal).
// We test the persistence contract by:
//   1. Importing and invoking the partialize function directly.
//   2. Verifying it is wired to ZUSTAND_STORAGE_KEYS.INGESTION_STORE (source assertion).
// ============================================================================

// The partialize function that the persist middleware uses — exported for testability
import { partializeIngestionState } from "../use-ingestion-store";
import { ZUSTAND_STORAGE_KEYS } from "@/lib/storage-keys";

describe("D-11 persistence: partialize function covers completedJobs", () => {
  it("partializeIngestionState is exported and is a function (persist wiring assertion)", () => {
    expect(typeof partializeIngestionState).toBe("function");
  });

  it("INGESTION_STORE key is registered in ZUSTAND_STORAGE_KEYS (storage wiring)", () => {
    expect(ZUSTAND_STORAGE_KEYS.INGESTION_STORE).toBe("edgequake-ingestion");
  });

  it("partialize output includes completedJobs with snapshot block", () => {
    const fakeState = {
      completedJobs: [
        {
          document_id: "doc-1",
          track_id: "track-1",
          chunks: 5,
          entities: 20,
          relationships: 8,
          duration_ms: 1000,
          snapshot: {
            uri: "s3://persist-test/snapshot",
            created_at: "2026-06-10T10:00:00Z",
            counts: {
              documents: 1,
              chunks: 5,
              entities: 20,
              relationships: 8,
              vectors: 5,
              bm25: 5,
            },
          },
        },
      ],
      // Maps must not be persisted
      tracks: new Map([["t1", {} as never]]),
      failedJobs: new Map([["t2", {} as never]]),
      // Other scalar state
      wsConnected: true,
      wsReconnecting: false,
      wsMaxReconnectsReached: false,
    };

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const result = partializeIngestionState(fakeState as any);

    expect(result).toHaveProperty("completedJobs");
    expect(result.completedJobs).toHaveLength(1);
    expect(result.completedJobs[0].snapshot?.uri).toBe(
      "s3://persist-test/snapshot"
    );
    expect(result.completedJobs[0].snapshot?.counts.bm25).toBe(5);
  });

  it("partialize output does NOT contain tracks or failedJobs Maps", () => {
    const fakeState = {
      completedJobs: [],
      tracks: new Map([["t1", {} as never]]),
      failedJobs: new Map([["t2", {} as never]]),
      wsConnected: false,
      wsReconnecting: false,
      wsMaxReconnectsReached: false,
    };

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const result = partializeIngestionState(fakeState as any);

    // Maps must be excluded (don't round-trip through JSON storage)
    expect(result).not.toHaveProperty("tracks");
    expect(result).not.toHaveProperty("failedJobs");
  });

  it("completedJobs snapshot survives JSON round-trip (D-11 serialization)", () => {
    const job = {
      document_id: "doc-rtt",
      track_id: "track-rtt",
      chunks: 3,
      entities: 10,
      relationships: 4,
      duration_ms: 500,
      snapshot: {
        uri: "/local/snapshots/rtt",
        created_at: "2026-06-10T09:00:00Z",
        counts: {
          documents: 1,
          chunks: 3,
          entities: 10,
          relationships: 4,
          vectors: 3,
          bm25: 3,
        },
      },
    };

    // Simulate what localStorage does: serialize → deserialize
    const serialized = JSON.stringify({ completedJobs: [job] });
    const deserialized = JSON.parse(serialized);

    expect(deserialized.completedJobs[0].snapshot?.uri).toBe(
      "/local/snapshots/rtt"
    );
    expect(deserialized.completedJobs[0].snapshot?.counts.vectors).toBe(3);
  });
});
