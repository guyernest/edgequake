/**
 * Unit tests for useWizardStore Zustand slice (Phase 23 Plan 06 D-14).
 *
 * Tests the session-only wizard state: step navigation, schema retention,
 * and reset behaviour.
 */

import type { SchemaProposal } from "@/types/ingestion";
import { act } from "react";
import { beforeEach, describe, expect, it } from "vitest";
import { useWizardStore } from "../use-wizard-store";

// Reset store to initial state before each test to ensure test isolation.
beforeEach(() => {
  act(() => {
    useWizardStore.getState().reset();
  });
});

// ============================================================================
// Initial State
// ============================================================================

describe("initial state", () => {
  it("currentStep is 'propose'", () => {
    const { currentStep } = useWizardStore.getState();
    expect(currentStep).toBe("propose");
  });

  it("schema is null", () => {
    const { schema } = useWizardStore.getState();
    expect(schema).toBeNull();
  });

  it("previewResult is null", () => {
    const { previewResult } = useWizardStore.getState();
    expect(previewResult).toBeNull();
  });

  it("namespace is null", () => {
    const { namespace } = useWizardStore.getState();
    expect(namespace).toBeNull();
  });

  it("isPreviewing is false", () => {
    const { isPreviewing } = useWizardStore.getState();
    expect(isPreviewing).toBe(false);
  });
});

// ============================================================================
// Step Navigation
// ============================================================================

describe("setStep", () => {
  it("updates currentStep from 'propose' to 'review'", () => {
    act(() => {
      useWizardStore.getState().setStep("review");
    });
    expect(useWizardStore.getState().currentStep).toBe("review");
  });

  it("navigates forward through all steps", () => {
    const { setStep } = useWizardStore.getState();

    act(() => setStep("review"));
    expect(useWizardStore.getState().currentStep).toBe("review");

    act(() => setStep("preview"));
    expect(useWizardStore.getState().currentStep).toBe("preview");

    act(() => setStep("approve"));
    expect(useWizardStore.getState().currentStep).toBe("approve");
  });

  it("navigates backward via setStep", () => {
    act(() => {
      useWizardStore.getState().setStep("approve");
    });
    act(() => {
      useWizardStore.getState().setStep("preview");
    });
    expect(useWizardStore.getState().currentStep).toBe("preview");
  });
});

// ============================================================================
// Schema State
// ============================================================================

describe("setSchema", () => {
  it("stores a schema proposal", () => {
    const proposal: SchemaProposal = {
      status: "proposed",
      entity_types: [{ name: "PERSON", description: "A person", frequency: 5, is_baseline: false }],
      relation_types: [],
    };

    act(() => {
      useWizardStore.getState().setSchema(proposal);
    });

    expect(useWizardStore.getState().schema).toEqual(proposal);
  });

  it("schema is retained after setStep", () => {
    const proposal: SchemaProposal = {
      status: "proposed",
      entity_types: [{ name: "ORG", description: "An organisation", frequency: 3, is_baseline: false }],
      relation_types: [],
    };

    act(() => {
      useWizardStore.getState().setSchema(proposal);
      useWizardStore.getState().setStep("preview");
    });

    // Schema must survive step navigation (state not wiped on step change).
    expect(useWizardStore.getState().schema).toEqual(proposal);
    expect(useWizardStore.getState().currentStep).toBe("preview");
  });

  it("accepts null to clear schema", () => {
    const proposal: SchemaProposal = {
      status: "proposed",
      entity_types: [],
      relation_types: [],
    };

    act(() => {
      useWizardStore.getState().setSchema(proposal);
    });
    act(() => {
      useWizardStore.getState().setSchema(null);
    });

    expect(useWizardStore.getState().schema).toBeNull();
  });
});

// ============================================================================
// Reset
// ============================================================================

describe("reset", () => {
  it("returns currentStep to 'propose'", () => {
    act(() => {
      useWizardStore.getState().setStep("approve");
    });
    act(() => {
      useWizardStore.getState().reset();
    });
    expect(useWizardStore.getState().currentStep).toBe("propose");
  });

  it("clears schema on reset", () => {
    const proposal: SchemaProposal = {
      status: "proposed",
      entity_types: [{ name: "COMPANY", description: "A company", frequency: 7, is_baseline: false }],
      relation_types: [],
    };

    act(() => {
      useWizardStore.getState().setSchema(proposal);
      useWizardStore.getState().setStep("review");
    });
    act(() => {
      useWizardStore.getState().reset();
    });

    expect(useWizardStore.getState().schema).toBeNull();
    expect(useWizardStore.getState().currentStep).toBe("propose");
  });

  it("clears previewResult on reset", () => {
    act(() => {
      useWizardStore.getState().setPreviewResult({
        status: "completed",
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
      });
    });
    act(() => {
      useWizardStore.getState().reset();
    });
    expect(useWizardStore.getState().previewResult).toBeNull();
  });
});

// ============================================================================
// Other Actions
// ============================================================================

describe("setNamespace", () => {
  it("stores the namespace slug", () => {
    act(() => {
      useWizardStore.getState().setNamespace("my-workspace");
    });
    expect(useWizardStore.getState().namespace).toBe("my-workspace");
  });
});

describe("setIsPreviewing", () => {
  it("toggles isPreviewing flag", () => {
    act(() => {
      useWizardStore.getState().setIsPreviewing(true);
    });
    expect(useWizardStore.getState().isPreviewing).toBe(true);
  });
});
