/**
 * @module use-wizard-store
 * @description Zustand store for the ingestion wizard session state (D-14 Phase 23).
 *
 * Session-only — uses devtools but NOT persist. Wizard state resets on page reload
 * (schema proposals are server-side; this store tracks in-flight wizard UI only).
 *
 * @implements D-14 — Ingestion wizard propose → review → preview → approve lifecycle
 */

import type { PreviewResult, SchemaProposal } from "@/types/ingestion";
import { create } from "zustand";
import { devtools } from "zustand/middleware";

// ============================================================================
// Types
// ============================================================================

/** The four wizard steps in order. */
export type WizardStep = "propose" | "review" | "preview" | "approve";

export const WIZARD_STEPS: WizardStep[] = [
  "propose",
  "review",
  "preview",
  "approve",
] as const;

interface WizardState {
  /** Current active step. */
  currentStep: WizardStep;
  /** Resolved namespace slug for this wizard session (from the workspace). */
  namespace: string | null;
  /** The schema proposal returned/edited during review. */
  schema: SchemaProposal | null;
  /** The extraction preview aggregate result (set after preview step completes). */
  previewResult: PreviewResult | null;
  /** True while a preview run is in-flight. */
  isPreviewing: boolean;
}

interface WizardActions {
  setStep: (step: WizardStep) => void;
  setNamespace: (ns: string) => void;
  setSchema: (schema: SchemaProposal | null) => void;
  setPreviewResult: (result: PreviewResult | null) => void;
  setIsPreviewing: (v: boolean) => void;
  /** Reset all wizard state back to the initial propose step. */
  reset: () => void;
}

// ============================================================================
// Initial State
// ============================================================================

const initialState: WizardState = {
  currentStep: "propose",
  namespace: null,
  schema: null,
  previewResult: null,
  isPreviewing: false,
};

// ============================================================================
// Store Definition
// ============================================================================

export const useWizardStore = create<WizardState & WizardActions>()(
  devtools(
    (set) => ({
      ...initialState,

      setStep: (step) => set({ currentStep: step }, false, "setStep"),

      setNamespace: (ns) => set({ namespace: ns }, false, "setNamespace"),

      setSchema: (schema) => set({ schema }, false, "setSchema"),

      setPreviewResult: (result) =>
        set({ previewResult: result }, false, "setPreviewResult"),

      setIsPreviewing: (v) => set({ isPreviewing: v }, false, "setIsPreviewing"),

      reset: () => set({ ...initialState }, false, "reset"),
    }),
    { name: "wizard-store" },
  ),
);

export default useWizardStore;
