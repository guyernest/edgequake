/**
 * @module edgequake-schema.test
 * @description Contract tests for the schema-lifecycle API client functions.
 *
 * These tests exercise the REAL exported client functions against the
 * server's ACTUAL JSON envelope shape:
 *   GET/PATCH  → SchemaResponse        { namespace, schema: { status, ... } }
 *   approve/reject → SchemaActionResponse { namespace, schema: SchemaProposal }
 *
 * Regression guard for CR-01: the client must unwrap the envelope so
 * consumers (propose-step polling, review-step editor) read a flat
 * SchemaProposal with a top-level `status` field.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

// Mock the low-level api client; the real edgequake.ts functions run on top.
vi.mock("@/lib/api/client", () => {
  const api = {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    patch: vi.fn(),
    delete: vi.fn(),
  };
  return { api, default: api, SERVER_BASE_URL: "" };
});

import { api } from "@/lib/api/client";
import {
  approveNamespaceSchema,
  getNamespaceSchema,
  rejectNamespaceSchema,
  updateNamespaceSchema,
} from "@/lib/api/edgequake";

const mockedApi = vi.mocked(api);

/** The server's actual GET /schema response for a real proposal. */
const SERVER_PROPOSED_ENVELOPE = {
  namespace: "my-ns",
  schema: {
    status: "proposed",
    entity_types: [
      { name: "PERSON", description: "A named person", frequency: 10, is_baseline: true },
    ],
    relation_types: [
      {
        name: "employs",
        description: "employment",
        source_type: "ORG",
        target_type: "PERSON",
        frequency: 5,
      },
    ],
    sample_size: 5,
    total_documents: 100,
    domain_hint: "legal",
    proposed_at: 1700000000000,
    reviewed_at: null,
  },
};

beforeEach(() => {
  vi.clearAllMocks();
});

describe("getNamespaceSchema — unwraps the { namespace, schema } envelope", () => {
  it("returns the inner schema with a top-level status field", async () => {
    mockedApi.get.mockResolvedValueOnce(SERVER_PROPOSED_ENVELOPE);

    const result = await getNamespaceSchema("my-ns");

    expect(mockedApi.get).toHaveBeenCalledWith("/namespaces/my-ns/schema");
    expect(result).not.toBeNull();
    expect(result!.status).toBe("proposed");
    expect(result!.entity_types).toHaveLength(1);
    expect(result!.entity_types[0].name).toBe("PERSON");
    expect(result!.relation_types).toHaveLength(1);
    // The envelope's `namespace` field must NOT leak into the proposal
    expect(result).not.toHaveProperty("schema");
  });

  it("unwraps the proposing sentinel body", async () => {
    mockedApi.get.mockResolvedValueOnce({
      namespace: "my-ns",
      schema: { status: "proposing" },
    });

    const result = await getNamespaceSchema("my-ns");
    expect(result!.status).toBe("proposing");
  });

  it("unwraps the none body (no schema proposed)", async () => {
    mockedApi.get.mockResolvedValueOnce({
      namespace: "my-ns",
      schema: { status: "none" },
    });

    const result = await getNamespaceSchema("my-ns");
    expect(result!.status).toBe("none");
  });

  it("unwraps the failed body with its error message", async () => {
    mockedApi.get.mockResolvedValueOnce({
      namespace: "my-ns",
      schema: { status: "failed", error: "OPENAI_API_KEY not set" },
    });

    const result = await getNamespaceSchema("my-ns");
    expect(result!.status).toBe("failed");
    expect(result!.error).toBe("OPENAI_API_KEY not set");
  });

  it("returns null when the response is null", async () => {
    mockedApi.get.mockResolvedValueOnce(null);
    const result = await getNamespaceSchema("my-ns");
    expect(result).toBeNull();
  });
});

describe("updateNamespaceSchema — unwraps the PATCH envelope", () => {
  it("returns the inner schema, not the envelope", async () => {
    mockedApi.patch.mockResolvedValueOnce(SERVER_PROPOSED_ENVELOPE);

    const result = await updateNamespaceSchema("my-ns", {
      entity_types: SERVER_PROPOSED_ENVELOPE.schema.entity_types as never,
    });

    expect(mockedApi.patch).toHaveBeenCalledWith(
      "/namespaces/my-ns/schema",
      expect.any(Object),
    );
    expect(result.status).toBe("proposed");
    expect(result.relation_types).toHaveLength(1);
    expect(result).not.toHaveProperty("schema");
  });
});

describe("approveNamespaceSchema / rejectNamespaceSchema — unwrap SchemaActionResponse", () => {
  it("approve returns the inner SchemaProposal", async () => {
    mockedApi.post.mockResolvedValueOnce({
      namespace: "my-ns",
      schema: { ...SERVER_PROPOSED_ENVELOPE.schema, status: "approved", reviewed_at: 1700000001000 },
    });

    const result = await approveNamespaceSchema("my-ns");
    expect(mockedApi.post).toHaveBeenCalledWith("/namespaces/my-ns/schema/approve");
    expect(result.status).toBe("approved");
    expect(result.entity_types[0].name).toBe("PERSON");
  });

  it("reject returns the inner SchemaProposal", async () => {
    mockedApi.post.mockResolvedValueOnce({
      namespace: "my-ns",
      schema: { ...SERVER_PROPOSED_ENVELOPE.schema, status: "rejected", reviewed_at: 1700000001000 },
    });

    const result = await rejectNamespaceSchema("my-ns");
    expect(mockedApi.post).toHaveBeenCalledWith("/namespaces/my-ns/schema/reject");
    expect(result.status).toBe("rejected");
  });
});
