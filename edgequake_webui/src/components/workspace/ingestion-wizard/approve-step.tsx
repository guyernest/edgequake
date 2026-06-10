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
 *   3. On success: toast.success + reset wizard store (session over)
 *   4. Secondary action: reject the schema (rejectNamespaceSchema)
 *   5. Back button returns to preview step
 *
 * @implements T-23-03 (Elevation of Privilege): server enforces namespace ownership on
 *   every schema/approve route; wizard resolves namespace from caller's own workspace only.
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
  rebuildKnowledgeGraph,
  rejectNamespaceSchema,
} from "@/lib/api/edgequake";
import { useWizardStore } from "@/stores/use-wizard-store";
import { useMutation } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useRouter } from "next/navigation";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

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
  const { schema, reset } = useWizardStore();

  // ── Approve + run mutation ────────────────────────────────────────────────
  const approveMutation = useMutation({
    mutationFn: async () => {
      // Step 1: approve the schema (sets status = "approved" server-side)
      await approveNamespaceSchema(namespace);
      // Step 2: trigger a full ingestion run — rebuildKnowledgeGraph clears the
      // graph and forces re-extraction with the newly-approved schema.
      // reprocessAllDocuments is re-embedding-only and is WRONG after a schema change.
      await rebuildKnowledgeGraph(workspaceId, {
        force: true,
        rebuild_embeddings: true,
      });
    },
    onSuccess: () => {
      toast.success(t("wizard.approve.successToast"));
      reset();
      // Navigate back to workspace after successful run trigger
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
  const entityCount = schema?.entity_types?.length ?? 0;
  const relationCount = schema?.relation_types?.length ?? 0;

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
            {schema ? (
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
                {schema.entity_types.length > 0 && (
                  <div className="flex flex-wrap gap-1">
                    {schema.entity_types.slice(0, 8).map((et) => (
                      <Badge key={et.name} variant="secondary">
                        {et.name}
                      </Badge>
                    ))}
                    {schema.entity_types.length > 8 && (
                      <Badge variant="outline">
                        +{schema.entity_types.length - 8} more
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
          <Button
            onClick={() => approveMutation.mutate()}
            disabled={isLoading || !schema}
          >
            {approveMutation.isPending ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                {t("common.loading")}
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
