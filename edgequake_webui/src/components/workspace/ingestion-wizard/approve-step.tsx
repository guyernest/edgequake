/**
 * @module ApproveStep
 * @description Wizard Step 4 — Approve the schema and start a full ingestion run (D-14).
 *
 * Flow:
 *   1. Show a summary of the schema stored in useWizardStore (set during propose step)
 *   2. Primary CTA: approveNamespaceSchema(namespace) THEN rebuildKnowledgeGraph(workspaceId)
 *      - rebuildKnowledgeGraph clears all graph entities/relationships and forces re-ingestion
 *        with the newly-approved schema (DECIDED — Codex #6, edgequake.ts:394).
 *        reprocessAllDocuments is re-embedding-only and is WRONG for a schema change.
 *   3. On success: toast.success + startTracking (run tracking) + reset wizard store + navigate
 *   4. Secondary action: reject the schema (rejectNamespaceSchema)
 *   5. Back button returns to preview step
 *
 * @implements T-23-03 (Elevation of Privilege): server enforces namespace ownership on
 *   every schema/approve route; wizard resolves namespace from caller's own workspace only.
 * @implements T-24-03-02 (Information Disclosure): startTracking args use a non-secret
 *   synthetic id (`rebuild:<workspaceId>`) and a static label string.
 */

"use client";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  approveNamespaceSchema,
  getNamespaceSchema,
  rebuildKnowledgeGraph,
  rejectNamespaceSchema,
} from "@/lib/api/edgequake";
import { useIngestionStore } from "@/stores/use-ingestion-store";
import { useWizardStore } from "@/stores/use-wizard-store";
import type { RebuildKnowledgeGraphResponse } from "@/lib/api/edgequake";
import type { SchemaProposal } from "@/types/ingestion";
import { useMutation, useQuery } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useRouter } from "next/navigation";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

// ============================================================================
// Pure helpers — exported for unit testing (no React imports needed in tests)
// ============================================================================

/**
 * The synthetic document id passed to startTracking for a wizard-triggered rebuild.
 * Non-secret (just a scoping label, not a server id). T-24-03-02.
 */
export const REBUILD_TRACK_DOCUMENT_NAME = "Full knowledge graph rebuild";

/**
 * Return the (documentId, documentName) args for startTracking for a wizard rebuild.
 * documentId uses `rebuild:<workspaceId>` as a stable synthetic key.
 */
export function buildTrackingArgs(workspaceId: string): {
  documentId: string;
  documentName: string;
} {
  return {
    documentId: `rebuild:${workspaceId}`,
    documentName: REBUILD_TRACK_DOCUMENT_NAME,
  };
}

/**
 * Return the track_id from a rebuildKnowledgeGraph response, or undefined
 * when the server did not include one (optional field).
 */
export function resolveTrackId(
  rebuild: RebuildKnowledgeGraphResponse,
): string | undefined {
  return rebuild.track_id;
}

/**
 * CTA enablement gate — the Approve button is enabled when either:
 *   (a) the session-only wizard-store schema is "approved" (fresh wizard pass), OR
 *   (b) the fetched namespace schema (getNamespaceSchema) is "approved" (re-entry path).
 *
 * This is the concrete approved-schema source for the re-entry CTA: the session-only
 * wizard-store schema is null on re-entry, but the server-fetched schema carries
 * the persisted approved status (WR-09 / plan 24-03 root cause fix).
 */
export function isSchemaApproved(
  wizardSchema: SchemaProposal | null,
  fetchedSchema: SchemaProposal | null | undefined,
): boolean {
  return (
    wizardSchema?.status === "approved" || fetchedSchema?.status === "approved"
  );
}

// ============================================================================
// Component
// ============================================================================

interface ApproveStepProps {
  /** Resolved namespace slug (from resolveNamespaceSlug in the wizard route). */
  namespace: string;
  /** Workspace UUID — required for the workspaceId-scoped rebuildKnowledgeGraph trigger. */
  workspaceId: string;
  /** Return to the preview step. */
  onBack: () => void;
}

export function ApproveStep({ namespace, workspaceId, onBack }: ApproveStepProps) {
  const { t } = useTranslation();
  const router = useRouter();
  const { schema: wizardSchema, reset } = useWizardStore();

  // ── Fetch the persisted namespace schema (the re-entry CTA source) ────────
  // A one-shot fetch on mount: when the wizard is re-entered on an already-approved
  // namespace, wizardSchema (session-only Zustand slice) is null; the persisted
  // server schema carries the approved status. Mirrors the propose-step pattern.
  const { data: fetchedSchema } = useQuery({
    queryKey: ["namespaceSchema", namespace],
    queryFn: () => getNamespaceSchema(namespace),
    // Always run on mount for the re-entry path; no repeated polling needed here.
    enabled: true,
    staleTime: 0,
  });

  // ── CTA approved gate (re-entry fix — plan 24-03 root cause) ─────────────
  // Enabled when either source shows "approved": session wizard-store (fresh pass)
  // OR fetched namespace schema (re-entry with null wizard-store schema).
  const schemaApproved = isSchemaApproved(wizardSchema, fetchedSchema);

  // ── Approve + run mutation ────────────────────────────────────────────────
  const approveMutation = useMutation({
    mutationFn: async (): Promise<RebuildKnowledgeGraphResponse> => {
      // Step 1: approve the schema (sets status = "approved" server-side).
      // WR-09: when the wizard was re-entered with an ALREADY-approved schema
      // (from either the session store or the fetched namespace schema),
      // the server rejects re-approval with InvalidSchemaState — treat
      // already-approved as success and go straight to the run.
      const alreadyApproved =
        wizardSchema?.status === "approved" ||
        fetchedSchema?.status === "approved";
      if (!alreadyApproved) {
        await approveNamespaceSchema(namespace);
      }
      // Step 2: trigger a full ingestion run — rebuildKnowledgeGraph clears the
      // graph and forces re-extraction with the newly-approved schema.
      // reprocessAllDocuments is re-embedding-only and is WRONG after a schema change.
      const rebuild = await rebuildKnowledgeGraph(workspaceId, {
        force: true,
        rebuild_embeddings: true,
      });
      return rebuild;
    },
    onSuccess: (rebuild) => {
      // Step 3: register the run with the ingestion store so the run-report and
      // Recent Runs surfaces can track it (plan 24-03 run-tracking fix).
      // startTracking MUST be called BEFORE reset() and router.push so the store
      // is seeded while the component is still mounted. T-24-03-02: args are
      // non-secret (synthetic id + static label).
      const trackId = resolveTrackId(rebuild);
      if (trackId) {
        const { documentId, documentName } = buildTrackingArgs(workspaceId);
        useIngestionStore.getState().startTracking(trackId, documentId, documentName);
      } else {
        // track_id is optional on the server — surface a non-fatal warning and
        // continue navigating. The run is still triggered; just not UI-tracked.
        toast.warning(t("wizard.approve.notTrackedWarning"));
      }

      toast.success(t("wizard.approve.successToast"));
      reset();
      // Navigate back to workspace after successful run trigger.
      // WebSocketProvider is app-wide and stays connected after navigation;
      // the ingestion_completed message will fire for the registered trackId
      // (see plan 24-02 server emit + websocket-provider.tsx:66).
      router.push("/workspace");
    },
    onError: (error) => {
      toast.error(t("wizard.approve.errorHeading"), {
        description:
          error instanceof Error ? error.message : t("common.unknownError"),
      });
    },
  });

  // ── Reject mutation ───────────────────────────────────────────────────────
  const rejectMutation = useMutation({
    mutationFn: () => rejectNamespaceSchema(namespace),
    onSuccess: () => {
      toast.info(t("wizard.steps.propose"));
      reset();
      router.push("/workspace");
    },
    onError: (error) => {
      toast.error(t("common.error"), {
        description:
          error instanceof Error ? error.message : t("common.unknownError"),
      });
    },
  });

  const isLoading = approveMutation.isPending || rejectMutation.isPending;

  // ── Schema summary ────────────────────────────────────────────────────────
  // Use wizardSchema for the summary display; fetchedSchema fills in on re-entry.
  const displaySchema = wizardSchema ?? fetchedSchema ?? null;
  const entityCount = displaySchema?.entity_types?.length ?? 0;
  const relationCount = displaySchema?.relation_types?.length ?? 0;

  // ── Render ────────────────────────────────────────────────────────────────
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("wizard.approve.heading")}</CardTitle>
        <CardDescription>{t("wizard.approve.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        {/* Schema summary */}
        <div className="space-y-2">
          <p className="text-sm font-medium text-muted-foreground">
            {t("wizard.approve.schemaLabel")}
          </p>
          <div className="rounded-lg border bg-muted/50 p-4 space-y-3">
            {displaySchema ? (
              <>
                <div className="flex flex-wrap gap-2">
                  <div className="flex items-center gap-1.5">
                    <span className="text-sm text-muted-foreground">
                      {entityCount} entity{" "}
                      {entityCount === 1 ? "type" : "types"}
                    </span>
                  </div>
                  <div className="flex items-center gap-1.5">
                    <span className="text-sm text-muted-foreground">
                      {relationCount} relation{" "}
                      {relationCount === 1 ? "type" : "types"}
                    </span>
                  </div>
                </div>
                {/* Entity type badges */}
                {displaySchema.entity_types.length > 0 && (
                  <div className="flex flex-wrap gap-1">
                    {displaySchema.entity_types.slice(0, 8).map((et) => (
                      <Badge key={et.name} variant="secondary">
                        {et.name}
                      </Badge>
                    ))}
                    {displaySchema.entity_types.length > 8 && (
                      <Badge variant="outline">
                        +{displaySchema.entity_types.length - 8} more
                      </Badge>
                    )}
                  </div>
                )}
              </>
            ) : (
              <p className="text-sm text-muted-foreground">
                {t("wizard.approve.schemaLabel")}
              </p>
            )}
          </div>
        </div>

        {/* Error state */}
        {approveMutation.isError && (
          <div className="rounded-lg border border-destructive/50 bg-destructive/10 p-4 space-y-1">
            <p className="text-sm font-medium text-destructive">
              {t("wizard.approve.errorHeading")}
            </p>
            <p className="text-xs text-muted-foreground">
              {t("wizard.approve.errorBody")}
            </p>
          </div>
        )}

        {/* Actions */}
        <div className="flex flex-wrap gap-2">
          <Button
            variant="outline"
            onClick={onBack}
            disabled={isLoading}
          >
            {t("common.back")}
          </Button>

          {/* Reject action (secondary) */}
          <Button
            variant="ghost"
            onClick={() => rejectMutation.mutate()}
            disabled={isLoading}
            className="text-destructive hover:text-destructive"
          >
            {rejectMutation.isPending ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : null}
            {t("common.cancel")}
          </Button>

          {/* Primary CTA: approve + run */}
          {/* disabled gate: isLoading (in-flight prevents double-submit) OR !schemaApproved */}
          {/* schemaApproved = wizardSchema approved OR fetchedSchema approved (re-entry fix) */}
          <Button
            onClick={() => approveMutation.mutate()}
            disabled={isLoading || !schemaApproved}
          >
            {approveMutation.isPending ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                {t("wizard.approve.triggering")}
              </>
            ) : (
              t("wizard.approve.cta")
            )}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
