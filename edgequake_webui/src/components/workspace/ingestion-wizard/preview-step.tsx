/**
 * @module PreviewStep — Real implementation (Plan 07, replaces Plan 06 stub)
 * @description Per-step preview tabs (Chunks / Entities / Relations) with budget note
 * and RELATED_TO amber highlighting (D-16 Phase 23).
 *
 * Key behaviours:
 * - On mount: triggers preview via request/poll (triggerExtractionPreview then polls
 *   getPreviewResult until status=completed).
 * - Budget note ALWAYS visible: "Sampling up to 6 documents and 60 chunks."
 * - Three shadcn Tabs: chunks, entities, relations.
 * - Each tab shows Skeleton while the polled PREVIEW_RESULT is pending/running.
 * - All three tabs populate from ONE aggregate PREVIEW_RESULT (counts, not rows).
 * - Relations tab: any row whose typeName === 'RELATED_TO' receives the amber highlight
 *   (data-testid="related-to-fallback-row") + outline amber Badge.
 * - DATA IS AGGREGATE ONLY — no per-row chunk-text, entity descriptions, or relation rows.
 *
 * @implements D-16 — Preview Extraction wizard step
 * @see 23-02-SUMMARY.md for the PREVIEW_RESULT aggregate field contract
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
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  getPreviewResult,
  triggerExtractionPreview,
} from "@/lib/api/edgequake";
import { useWizardStore } from "@/stores/use-wizard-store";
import type { PreviewResult } from "@/types/ingestion";
import { useMutation } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

// ============================================================================
// Constants
// ============================================================================

/** The system-fallback relation type name. */
const RELATED_TO = "RELATED_TO";

/** Poll interval in ms when the preview is running. */
const POLL_INTERVAL_MS = 3000;

/** Maximum number of polls before giving up (5 min at 3s interval). */
const MAX_POLLS = 100;

// ============================================================================
// Pure helpers (logic tested in __tests__/preview-step.test.tsx)
// ============================================================================

/**
 * Returns true when a relation type count row is the RELATED_TO fallback.
 * These rows receive the amber highlight wrapper + fallback badge.
 */
export function isRelatedToRow(typeName: string): boolean {
  return typeName === RELATED_TO;
}

/**
 * Returns true when the preview is still loading (no terminal result yet).
 * In-flight server statuses are "requested" and "processing"; "none" means
 * no request is recorded yet (treated as still loading while a poll is
 * expected). Terminal statuses are "completed" and "failed" (CR-05).
 */
export function isPreviewLoading(
  isPreviewing: boolean,
  result: PreviewResult | null,
): boolean {
  if (isPreviewing) return true;
  if (!result) return false;
  return (
    result.status === "requested" ||
    result.status === "processing" ||
    result.status === "none"
  );
}

// ============================================================================
// Skeleton rows helper
// ============================================================================

function SkeletonRows() {
  return (
    <>
      <TableRow>
        <TableCell><Skeleton className="h-4 w-32" /></TableCell>
        <TableCell><Skeleton className="h-4 w-16" /></TableCell>
      </TableRow>
      <TableRow>
        <TableCell><Skeleton className="h-4 w-24" /></TableCell>
        <TableCell><Skeleton className="h-4 w-12" /></TableCell>
      </TableRow>
      <TableRow>
        <TableCell><Skeleton className="h-4 w-40" /></TableCell>
        <TableCell><Skeleton className="h-4 w-20" /></TableCell>
      </TableRow>
    </>
  );
}

// ============================================================================
// Main component
// ============================================================================

interface PreviewStepProps {
  namespace: string;
  onNext: () => void;
  onBack: () => void;
}

export function PreviewStep({ namespace, onNext, onBack }: PreviewStepProps) {
  const { t } = useTranslation();
  const { previewResult, isPreviewing, setPreviewResult, setIsPreviewing } =
    useWizardStore();

  const [error, setError] = useState<string | null>(null);
  const pollIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const pollCountRef = useRef(0);

  // ── Poll helper ────────────────────────────────────────────────────────────
  const startPolling = () => {
    pollCountRef.current = 0;
    pollIntervalRef.current = setInterval(async () => {
      pollCountRef.current += 1;
      if (pollCountRef.current > MAX_POLLS) {
        stopPolling();
        setIsPreviewing(false);
        setError(t("wizard.preview.emptyChunks")); // generic error fallback
        return;
      }
      try {
        const result = await getPreviewResult(namespace);
        if (result.status === "completed" || result.status === "failed") {
          stopPolling();
          // Only TERMINAL bodies are committed to the store — in-flight
          // poll bodies ({ namespace, status }) have none of the aggregate
          // fields and must not be stored as a PreviewResult (CR-05).
          setPreviewResult(result);
          setIsPreviewing(false);
          if (result.status === "failed") {
            setError(result.error ?? t("common.error"));
          }
        }
        // Still requested/processing — keep polling without storing partials
      } catch (err) {
        stopPolling();
        setIsPreviewing(false);
        setError(err instanceof Error ? err.message : t("common.error"));
      }
    }, POLL_INTERVAL_MS);
  };

  const stopPolling = () => {
    if (pollIntervalRef.current) {
      clearInterval(pollIntervalRef.current);
      pollIntervalRef.current = null;
    }
  };

  // ── Trigger preview mutation ───────────────────────────────────────────────
  const triggerMutation = useMutation({
    mutationFn: () => triggerExtractionPreview(namespace),
    onSuccess: () => {
      setIsPreviewing(true);
      startPolling();
    },
    onError: (err) => {
      toast.error(t("common.error"), {
        description:
          err instanceof Error ? err.message : t("common.unknownError"),
      });
      setError(err instanceof Error ? err.message : t("common.error"));
    },
  });

  // ── Auto-trigger on mount if no result yet ─────────────────────────────────
  useEffect(() => {
    if (!previewResult && !isPreviewing && !triggerMutation.isPending) {
      triggerMutation.mutate();
    }
    return () => stopPolling();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ── Derived state ──────────────────────────────────────────────────────────
  const loading = isPreviewLoading(isPreviewing, previewResult);
  const result = previewResult;

  // Guarded aggregate views — a "failed" (or malformed) result carries no
  // aggregate arrays; never dereference them unguarded (CR-05).
  const documentColumns = result?.documentColumns ?? [];
  const coverageRows = result?.coverageRows ?? [];
  const entityTypeCounts = result?.entityTypeCounts ?? [];
  const relationTypeCounts = result?.relationTypeCounts ?? [];

  // ── Render ─────────────────────────────────────────────────────────────────
  return (
    <TooltipProvider>
      <Card>
        <CardHeader>
          <CardTitle>{t("wizard.preview.heading")}</CardTitle>
          <CardDescription>{t("wizard.preview.description")}</CardDescription>
          {/* Budget note — ALWAYS visible (D-16) */}
          <p className="text-sm text-muted-foreground mt-1">
            {t("wizard.preview.budget")}
          </p>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* ── Loading indicator while trigger is pending ─────────────────── */}
          {(triggerMutation.isPending || loading) && (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              <span>{t("wizard.preview.loading", { step: "extraction" })}</span>
            </div>
          )}

          {/* ── Error state ────────────────────────────────────────────────── */}
          {error && (
            <p className="text-sm text-destructive">{error}</p>
          )}

          {/* ── Tabs ──────────────────────────────────────────────────────── */}
          <Tabs defaultValue="chunks">
            <TabsList>
              <TabsTrigger value="chunks">
                {t("wizard.preview.tabs.chunks")}
              </TabsTrigger>
              <TabsTrigger value="entities">
                {t("wizard.preview.tabs.entities")}
              </TabsTrigger>
              <TabsTrigger value="relations">
                {t("wizard.preview.tabs.relations")}
              </TabsTrigger>
            </TabsList>

            {/* ── Chunks tab ──────────────────────────────────────────────── */}
            <TabsContent value="chunks">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("common.name")}</TableHead>
                    <TableHead>{t("wizard.preview.tabs.chunks")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {loading ? (
                    <SkeletonRows />
                  ) : result && documentColumns.length > 0 ? (
                    <>
                      {documentColumns.map((doc) => {
                        // Find chunk count from coverageRows totals per document
                        const colIdx = documentColumns.findIndex(
                          (d) => d.id === doc.id,
                        );
                        const chunkCount =
                          colIdx >= 0
                            ? coverageRows.reduce(
                                (sum, row) => sum + (row.counts[colIdx] ?? 0),
                                0,
                              )
                            : 0;
                        return (
                          <TableRow key={doc.id}>
                            <TableCell className="text-xs" title={doc.name}>
                              {doc.truncatedName}
                            </TableCell>
                            <TableCell className="text-xs tabular-nums">
                              {chunkCount}
                            </TableCell>
                          </TableRow>
                        );
                      })}
                      <TableRow className="font-medium">
                        <TableCell className="text-xs">
                          {t("common.all")}
                        </TableCell>
                        <TableCell className="text-xs tabular-nums">
                          {result.totalChunks ?? 0}
                        </TableCell>
                      </TableRow>
                    </>
                  ) : (
                    <TableRow>
                      <TableCell
                        colSpan={2}
                        className="text-xs text-muted-foreground text-center py-4"
                      >
                        {t("wizard.preview.emptyChunks")}
                      </TableCell>
                    </TableRow>
                  )}
                </TableBody>
              </Table>
            </TabsContent>

            {/* ── Entities tab ────────────────────────────────────────────── */}
            <TabsContent value="entities">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("common.name")}</TableHead>
                    <TableHead>{t("common.count") ?? "Count"}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {loading ? (
                    <SkeletonRows />
                  ) : result && entityTypeCounts.length > 0 ? (
                    <>
                      {entityTypeCounts.map((row) => (
                        <TableRow key={row.typeName}>
                          <TableCell className="text-xs font-mono">
                            {row.typeName}
                          </TableCell>
                          <TableCell className="text-xs tabular-nums">
                            {row.count}
                          </TableCell>
                        </TableRow>
                      ))}
                      <TableRow className="font-medium">
                        <TableCell className="text-xs">
                          {t("common.all")}
                        </TableCell>
                        <TableCell className="text-xs tabular-nums">
                          {result.totalEntities ?? 0}
                        </TableCell>
                      </TableRow>
                    </>
                  ) : (
                    <TableRow>
                      <TableCell
                        colSpan={2}
                        className="text-xs text-muted-foreground text-center py-4"
                      >
                        {t("wizard.preview.emptyEntities")}
                      </TableCell>
                    </TableRow>
                  )}
                </TableBody>
              </Table>
            </TabsContent>

            {/* ── Relations tab ───────────────────────────────────────────── */}
            <TabsContent value="relations">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("common.name")}</TableHead>
                    <TableHead>{t("common.count") ?? "Count"}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {loading ? (
                    <SkeletonRows />
                  ) : result && relationTypeCounts.length > 0 ? (
                    <>
                      {relationTypeCounts.map((row) => {
                        const isRT = isRelatedToRow(row.typeName);
                        return (
                          <TableRow
                            key={row.typeName}
                            // Amber highlight for RELATED_TO fallback rows (D-16 / UI-SPEC Color)
                            className={
                              isRT
                                ? "bg-amber-50 dark:bg-amber-950/20"
                                : undefined
                            }
                            data-testid={
                              isRT ? "related-to-fallback-row" : undefined
                            }
                          >
                            <TableCell className="text-xs font-mono">
                              <div className="flex items-center gap-1.5">
                                {row.typeName}
                                {isRT && (
                                  <Tooltip>
                                    <TooltipTrigger asChild>
                                      <Badge
                                        variant="outline"
                                        className="text-amber-600 dark:text-amber-400 border-amber-300 dark:border-amber-700 text-xs cursor-help"
                                      >
                                        {t("wizard.preview.relatedToFallback")}
                                      </Badge>
                                    </TooltipTrigger>
                                    <TooltipContent className="max-w-xs">
                                      {t("wizard.preview.relatedToTooltip")}
                                    </TooltipContent>
                                  </Tooltip>
                                )}
                              </div>
                            </TableCell>
                            <TableCell className="text-xs tabular-nums">
                              {row.count}
                            </TableCell>
                          </TableRow>
                        );
                      })}
                      <TableRow className="font-medium">
                        <TableCell className="text-xs">
                          {t("common.all")}
                        </TableCell>
                        <TableCell className="text-xs tabular-nums">
                          {result.totalRelationships ?? 0}
                        </TableCell>
                      </TableRow>
                    </>
                  ) : (
                    <TableRow>
                      <TableCell
                        colSpan={2}
                        className="text-xs text-muted-foreground text-center py-4"
                      >
                        {t("wizard.preview.emptyRelations")}
                      </TableCell>
                    </TableRow>
                  )}
                </TableBody>
              </Table>
            </TabsContent>
          </Tabs>

          {/* ── Navigation ────────────────────────────────────────────────── */}
          <div className="flex gap-2">
            <Button variant="outline" onClick={onBack} disabled={loading}>
              {t("common.back")}
            </Button>
            <Button
              onClick={onNext}
              disabled={loading || !result || result.status !== "completed"}
            >
              {t("wizard.preview.cta")}
            </Button>
          </div>
        </CardContent>
      </Card>
    </TooltipProvider>
  );
}
