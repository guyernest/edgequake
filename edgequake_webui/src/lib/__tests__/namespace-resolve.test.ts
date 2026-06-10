/**
 * @module namespace-resolve.test
 * @description Tests for the resolveNamespaceSlug function.
 *
 * Wave-0 gate: workspace.namespace_slug is populated server-side by Plan 01 Task 3
 * (WorkspaceResponse.namespace_slug = workspace.slug.clone()). CASE A applies —
 * the resolver reads the field directly; no runtime probe is needed.
 */

import { describe, expect, it } from "vitest";
import { resolveNamespaceSlug, NamespaceSlugError } from "../namespace-resolve";
import type { Workspace } from "@/types";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeWorkspace(overrides?: Partial<Workspace>): Workspace {
  return {
    id: "ws-uuid-1",
    tenant_id: "tenant-1",
    name: "Test Workspace",
    created_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("resolveNamespaceSlug", () => {
  describe("happy path — namespace_slug populated (CASE A)", () => {
    it("returns namespace_slug when present", () => {
      const ws = makeWorkspace({ namespace_slug: "my-workspace" });
      expect(resolveNamespaceSlug(ws)).toBe("my-workspace");
    });

    it("returns namespace_slug even when slug is also present", () => {
      const ws = makeWorkspace({ namespace_slug: "ns-slug", slug: "ws-slug" });
      expect(resolveNamespaceSlug(ws)).toBe("ns-slug");
    });

    it("handles slug with hyphens and underscores", () => {
      const ws = makeWorkspace({ namespace_slug: "football-context_v2" });
      expect(resolveNamespaceSlug(ws)).toBe("football-context_v2");
    });
  });

  describe("error path — no resolvable slug", () => {
    it("throws NamespaceSlugError when namespace_slug is absent", () => {
      const ws = makeWorkspace(); // no namespace_slug
      expect(() => resolveNamespaceSlug(ws)).toThrow(NamespaceSlugError);
    });

    it("throws NamespaceSlugError when namespace_slug is empty string", () => {
      const ws = makeWorkspace({ namespace_slug: "" });
      expect(() => resolveNamespaceSlug(ws)).toThrow(NamespaceSlugError);
    });

    it("error message includes workspace id", () => {
      const ws = makeWorkspace({ id: "ws-abc-123" });
      try {
        resolveNamespaceSlug(ws);
      } catch (e) {
        expect(e).toBeInstanceOf(NamespaceSlugError);
        expect((e as NamespaceSlugError).message).toContain("ws-abc-123");
      }
    });
  });

  describe("NamespaceSlugError properties", () => {
    it("is an instance of Error", () => {
      const err = new NamespaceSlugError("ws-id-1");
      expect(err).toBeInstanceOf(Error);
    });

    it("has descriptive name", () => {
      const err = new NamespaceSlugError("ws-id-1");
      expect(err.name).toBe("NamespaceSlugError");
    });
  });
});
