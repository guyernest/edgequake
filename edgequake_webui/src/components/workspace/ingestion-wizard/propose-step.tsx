/**
 * @module ProposeStep
 * @description Wizard Step 1 — Trigger schema suggestion and poll until proposal ready (D-14).
 *
 * Flow:
 *   1. User clicks "Propose schema" → calls proposeNamespaceSchema (POST /schema → 202)
 *   2. Polls getNamespaceSchema until status reaches a TERMINAL state:
 *      - "proposed"  → schema ready, advance to review step
 *      - "approved"  → schema already approved, also treat as terminal
 *      - "rejected"  → treat as terminal (user can re-propose)
 *      - "failed"    → show error, offer retry
 *   3. NEVER advance on "proposing" (in-progress sentinel, Plan 02 §Pending-Suggest-Sentinel)
 *   4. NEVER advance on "none" (no schema proposed yet)
 *
 * @implements T-23-10 (DoS): poll uses React Query refetchInterval with stop condition;
 *   poll stops as soon as a terminal status is reached (no indefinite polling).
 *   Max poll duration is bounded by the user closing the wizard; error state shown on
 *   HTTP failure so the user can retry.
 */

"use client";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  approveNamespaceSchema as _approveNamespaceSchema,
  getNamespaceSchema,
  proposeNamespaceSchema,
} from "@/lib/api/edgequake";
import { useWizardStore } from "@/stores/use-wizard-store";
import type { SchemaProposal } from "@/types/ingestion";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

// ============================================================================
// Poll stop condition
// ============================================================================

/** Terminal statuses that indicate the proposal is ready (or definitively failed). */
const TERMINAL_STATUSES: SchemaProposal["status"][] = [
  "proposed",
  "approved",
  "rejected",
  "failed",
];

/**
 * Maximum number of polls before giving up (10 min at 3s interval).
 * Hard cap so a stuck non-terminal status (e.g. "none" after a marker is
 * lost server-side) cannot poll forever (CR-02 / T-23-10).
 */
const MAX_POLLS = 200;

function isTerminal(status: SchemaProposal["status"]): boolean {
  return TERMINAL_STATUSES.includes(status);
}

// ============================================================================
// Component
// ============================================================================

interface ProposeStepProps {
  namespace: string;
  onNext: () => void;
}

export function ProposeStep({ namespace, onNext }: ProposeStepProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { setSchema } = useWizardStore();

  // Poll bookkeeping: hard cap + timeout surfacing (CR-02 / T-23-10)
  const pollCountRef = useRef(0);
  const [pollTimedOut, setPollTimedOut] = useState(false);

  // ── Propose mutation ──────────────────────────────────────────────────────
  const proposeMutation = useMutation({
    mutationFn: () => proposeNamespaceSchema(namespace),
    onSuccess: () => {
      // Schema suggestion is async (spawns a tokio task server-side).
      // Invalidate the schema query so the poll starts immediately.
      pollCountRef.current = 0;
      setPollTimedOut(false);
      queryClient.invalidateQueries({
        queryKey: ["namespaceSchema", namespace],
      });
    },
    onError: (error) => {
      toast.error(t("wizard.propose.errorHeading"), {
        description: error instanceof Error ? error.message : t("common.unknownError"),
      });
    },
  });

  const isProposing = proposeMutation.isPending;

  // ── Poll getNamespaceSchema ───────────────────────────────────────────────
  // Active only while a proposal is in-flight (proposeMutation succeeded).
  // Stops polling as soon as the status becomes terminal.
  const {
    data: schemaData,
    isLoading: isPolling,
    error: pollError,
  } = useQuery({
    queryKey: ["namespaceSchema", namespace],
    queryFn: () => getNamespaceSchema(namespace),
    enabled: proposeMutation.isSuccess || proposeMutation.isPending,
    // Poll every 3 seconds while status is not terminal, capped at MAX_POLLS.
    refetchInterval: (query) => {
      const status = query.state.data?.status;
      if (!status || isTerminal(status)) return false;
      pollCountRef.current += 1;
      if (pollCountRef.current > MAX_POLLS) {
        // Stop the poll and surface a retryable error (no indefinite polling)
        setPollTimedOut(true);
        return false;
      }
      return 3000;
    },
    staleTime: 0,
  });

  // ── Advance on terminal proposal ─────────────────────────────────────────
  useEffect(() => {
    if (!schemaData) return;
    if (schemaData.status === "proposing" || schemaData.status === "none") return;

    if (schemaData.status === "proposed" || schemaData.status === "approved") {
      // Store the schema in the wizard and advance.
      setSchema(schemaData);
      onNext();
    } else if (schemaData.status === "failed") {
      toast.error(t("wizard.propose.errorHeading"), {
        description: schemaData.error ?? t("wizard.propose.errorBody"),
      });
    }
    // "rejected" — show no toast (user explicitly rejected; they can re-propose)
  }, [schemaData, setSchema, onNext, t]);

  // ── Render ────────────────────────────────────────────────────────────────
  const showPolling =
    proposeMutation.isSuccess &&
    !pollTimedOut &&
    schemaData?.status !== undefined &&
    !isTerminal(schemaData.status);

  const hasError =
    pollError !== null || proposeMutation.isError || pollTimedOut;

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("wizard.propose.heading")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">
          {t("wizard.propose.description")}
        </p>

        {/* Polling state */}
        {showPolling && (
          <div className="flex flex-col gap-2 rounded-lg border bg-muted/50 p-4">
            <div className="flex items-center gap-2">
              <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
              <span className="text-sm font-medium">
                {t("wizard.propose.loading")}
              </span>
            </div>
            <p className="text-xs text-muted-foreground">
              {t("wizard.propose.pollNote")}
            </p>
          </div>
        )}

        {/* Error state */}
        {hasError && !showPolling && (
          <div className="rounded-lg border border-destructive/50 bg-destructive/10 p-4 space-y-1">
            <p className="text-sm font-medium text-destructive">
              {t("wizard.propose.errorHeading")}
            </p>
            <p className="text-xs text-muted-foreground">
              {t("wizard.propose.errorBody")}
            </p>
          </div>
        )}

        {/* CTA button */}
        {!showPolling && (
          <Button
            onClick={() => proposeMutation.mutate()}
            disabled={isProposing || isPolling}
          >
            {isProposing ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                {t("wizard.propose.loading")}
              </>
            ) : hasError ? (
              <>
                <RefreshCw className="mr-2 h-4 w-4" />
                {t("common.tryAgain")}
              </>
            ) : (
              t("wizard.propose.cta")
            )}
          </Button>
        )}
      </CardContent>
    </Card>
  );
}
