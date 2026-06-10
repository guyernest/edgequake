/**
 * @module IngestionWizard
 * @description 4-step stepper shell for the ingestion wizard (D-14 Phase 23).
 *
 * Renders a numbered step indicator and the active step component.
 * The step state lives in useWizardStore (session-only Zustand slice).
 *
 * Steps: propose → review → preview → approve
 *
 * Props:
 *   namespace   — resolved namespace slug (from resolveNamespaceSlug)
 *   workspaceId — workspace UUID (passed to ApproveStep for rebuildKnowledgeGraph)
 *
 * Review/Preview steps are STUBS in Plan 06 — replaced by Plan 07.
 */

"use client";

import { Separator } from "@/components/ui/separator";
import { useWizardStore, WIZARD_STEPS } from "@/stores/use-wizard-store";
import type { WizardStep } from "@/stores/use-wizard-store";
import { cn } from "@/lib/utils";
import React from "react";
import { useTranslation } from "react-i18next";
import { ApproveStep } from "./approve-step";
import { PreviewStep } from "./preview-step";
import { ProposeStep } from "./propose-step";
import { ReviewStep } from "./review-step";

// ============================================================================
// Types
// ============================================================================

interface IngestionWizardProps {
  /** Resolved namespace slug (from resolveNamespaceSlug in the wizard route). */
  namespace: string;
  /** Workspace UUID — needed by ApproveStep to call rebuildKnowledgeGraph. */
  workspaceId: string;
}

// ============================================================================
// Step Indicator
// ============================================================================

interface StepDotProps {
  index: number;
  step: WizardStep;
  currentStep: WizardStep;
}

function StepDot({ index, step, currentStep }: StepDotProps) {
  const { t } = useTranslation();
  const stepIndex = WIZARD_STEPS.indexOf(step);
  const currentIndex = WIZARD_STEPS.indexOf(currentStep);

  const isActive = step === currentStep;
  const isCompleted = stepIndex < currentIndex;

  return (
    <div className="flex flex-col items-center gap-1">
      <div
        className={cn(
          "flex h-8 w-8 items-center justify-center rounded-full text-sm font-medium transition-colors",
          isActive && "bg-primary text-primary-foreground",
          isCompleted && "bg-green-500 text-white",
          !isActive && !isCompleted && "bg-muted text-muted-foreground",
        )}
        aria-current={isActive ? "step" : undefined}
      >
        {isCompleted ? (
          // Checkmark for completed steps
          <svg
            className="h-4 w-4"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth={2.5}
            aria-hidden="true"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M5 13l4 4L19 7"
            />
          </svg>
        ) : (
          index + 1
        )}
      </div>
      <span
        className={cn(
          "text-xs font-medium",
          isActive ? "text-foreground" : "text-muted-foreground",
        )}
      >
        {t(`wizard.steps.${step}`)}
      </span>
    </div>
  );
}

// ============================================================================
// Stepper
// ============================================================================

interface StepperProps {
  currentStep: WizardStep;
}

function Stepper({ currentStep }: StepperProps) {
  return (
    <div className="flex items-start gap-0">
      {WIZARD_STEPS.map((step, i) => (
        <React.Fragment key={step}>
          <StepDot index={i} step={step} currentStep={currentStep} />
          {i < WIZARD_STEPS.length - 1 && (
            <div className="mt-4 flex-1 px-1">
              <Separator className="h-0.5" />
            </div>
          )}
        </React.Fragment>
      ))}
    </div>
  );
}

// ============================================================================
// IngestionWizard
// ============================================================================

export function IngestionWizard({ namespace, workspaceId }: IngestionWizardProps) {
  const { currentStep, setStep } = useWizardStore();
  const { t } = useTranslation();

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="space-y-1">
        <h1 className="text-xl font-medium">{t("wizard.title")}</h1>
        <p className="text-sm text-muted-foreground">{t("wizard.subtitle")}</p>
      </div>

      {/* Step indicator */}
      <Stepper currentStep={currentStep} />

      {/* Active step */}
      {currentStep === "propose" && (
        <ProposeStep
          namespace={namespace}
          onNext={() => setStep("review")}
        />
      )}
      {currentStep === "review" && (
        <ReviewStep
          namespace={namespace}
          onNext={() => setStep("preview")}
          onBack={() => setStep("propose")}
        />
      )}
      {currentStep === "preview" && (
        <PreviewStep
          namespace={namespace}
          onNext={() => setStep("approve")}
          onBack={() => setStep("review")}
        />
      )}
      {currentStep === "approve" && (
        <ApproveStep
          namespace={namespace}
          workspaceId={workspaceId}
          onBack={() => setStep("preview")}
        />
      )}
    </div>
  );
}
