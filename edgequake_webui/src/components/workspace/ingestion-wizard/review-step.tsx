/**
 * @module ReviewStep — Real implementation (Plan 07, replaces Plan 06 stub)
 * @description Editable entity and relation type lists with a fixed non-removable
 * RELATED_TO fallback row (D-15 Phase 23).
 *
 * Key behaviours:
 * - Entity types: editable name (UPPER_SNAKE_CASE on blur) + description rows.
 * - Relation types: editable rows with optional source/target constraints.
 * - Fixed RELATED_TO row rendered OUTSIDE the editable list, with Lock icon,
 *   dashed muted border, and NO delete button (T-23-04 / Pitfall 3).
 * - Save / proceed: filters RELATED_TO from payload before updateNamespaceSchema,
 *   writes filtered schema to wizard store, then calls onNext().
 *
 * @implements D-15 — Review Schema wizard step
 * @implements T-23-04 — RELATED_TO never sent to server
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
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  getNamespaceSchema,
  updateNamespaceSchema,
} from "@/lib/api/edgequake";
import { useWizardStore } from "@/stores/use-wizard-store";
import type {
  EntityTypeProposal,
  RelationTypeProposal,
} from "@/types/ingestion";
import { useMutation, useQuery } from "@tanstack/react-query";
import { Lock, Loader2, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

// ============================================================================
// Constants
// ============================================================================

/** The sentinel name for the system-fallback relation type (never sent to server). */
const RELATED_TO = "RELATED_TO";

/**
 * Sentinel Select value representing "no constraint" (CR-04).
 * Radix UI throws when a <Select.Item> has an empty-string value, so the
 * "none" option uses this sentinel and maps to `undefined` on save.
 */
const NONE_TYPE_SENTINEL = "__any__";

// ============================================================================
// Pure helpers (logic tested in __tests__/review-step.test.tsx)
// ============================================================================

/**
 * Returns true when a relation type row is the fixed RELATED_TO fallback.
 * The delete button must be ABSENT (not rendered) for this row.
 */
export function isFixedRelationRow(name: string): boolean {
  return name === RELATED_TO;
}

/**
 * Returns true when the given row should have a delete button.
 */
export function hasDeleteButton(name: string): boolean {
  return !isFixedRelationRow(name);
}

/**
 * Filters the relation_types array to exclude RELATED_TO before sending to
 * updateNamespaceSchema. Defense-in-depth for T-23-04 / Pitfall 3.
 */
export function filterRelatedTo(
  relationTypes: RelationTypeProposal[],
): RelationTypeProposal[] {
  return relationTypes.filter((r) => r.name !== RELATED_TO);
}

/**
 * Normalises an entity type name to UPPER_SNAKE_CASE on blur.
 * Rule (REVIEW Gemini #9): toUpperCase, replace non-alphanumeric runs with _,
 * strip leading/trailing underscores.
 */
export function normalizeEntityName(raw: string): string {
  return raw
    .toUpperCase()
    .replace(/[^A-Z0-9]+/g, "_")
    .replace(/^_|_$/g, "");
}

// ============================================================================
// Sub-components
// ============================================================================

interface DeleteConfirmDialogProps {
  open: boolean;
  name: string;
  onConfirm: () => void;
  onCancel: () => void;
}

function DeleteConfirmDialog({
  open,
  name,
  onConfirm,
  onCancel,
}: DeleteConfirmDialogProps) {
  const { t } = useTranslation();
  return (
    <Dialog open={open} onOpenChange={(v) => !v && onCancel()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("wizard.review.deleteConfirmTitle")}</DialogTitle>
          <DialogDescription>
            {t("wizard.review.deleteConfirmBody", { name })}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            {t("common.cancel")}
          </Button>
          <Button variant="destructive" onClick={onConfirm}>
            {t("wizard.review.deleteConfirmCta")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ============================================================================
// Entity type row
// ============================================================================

interface EntityRowProps {
  item: EntityTypeProposal;
  onUpdate: (updated: EntityTypeProposal) => void;
  onDelete: () => void;
}

function EntityRow({ item, onUpdate, onDelete }: EntityRowProps) {
  const { t } = useTranslation();
  const [deleteOpen, setDeleteOpen] = useState(false);

  return (
    <div className="flex items-start gap-2 p-2 border rounded-md bg-muted/20">
      <div className="flex-1 grid grid-cols-2 gap-2">
        <div className="space-y-1">
          <Label className="text-xs">{t("common.name")}</Label>
          <Input
            value={item.name}
            placeholder={t("wizard.review.namePlaceholderEntity")}
            onChange={(e) => onUpdate({ ...item, name: e.target.value })}
            onBlur={(e) =>
              onUpdate({ ...item, name: normalizeEntityName(e.target.value) })
            }
            className="h-7 text-xs font-mono"
          />
        </div>
        <div className="space-y-1">
          <Label className="text-xs">{t("common.description")}</Label>
          <Input
            value={item.description}
            placeholder={t("common.description")}
            onChange={(e) => onUpdate({ ...item, description: e.target.value })}
            className="h-7 text-xs"
          />
        </div>
      </div>
      <Button
        variant="ghost"
        size="icon"
        className="h-7 w-7 mt-4 text-destructive hover:text-destructive"
        aria-label={t("wizard.review.deleteAriaLabel", { name: item.name })}
        onClick={() => setDeleteOpen(true)}
      >
        <Trash2 className="h-3 w-3" />
      </Button>
      <DeleteConfirmDialog
        open={deleteOpen}
        name={item.name}
        onConfirm={() => {
          setDeleteOpen(false);
          onDelete();
        }}
        onCancel={() => setDeleteOpen(false)}
      />
    </div>
  );
}

// ============================================================================
// Relation type row (editable)
// ============================================================================

interface RelationRowProps {
  item: RelationTypeProposal;
  entityTypeNames: string[];
  onUpdate: (updated: RelationTypeProposal) => void;
  onDelete: () => void;
}

function RelationRow({
  item,
  entityTypeNames,
  onUpdate,
  onDelete,
}: RelationRowProps) {
  const { t } = useTranslation();
  const [deleteOpen, setDeleteOpen] = useState(false);

  return (
    <div className="flex items-start gap-2 p-2 border rounded-md bg-muted/20">
      <div className="flex-1 space-y-2">
        <div className="grid grid-cols-2 gap-2">
          <div className="space-y-1">
            <Label className="text-xs">{t("common.name")}</Label>
            <Input
              value={item.name}
              placeholder={t("wizard.review.namePlaceholderRelation")}
              onChange={(e) => onUpdate({ ...item, name: e.target.value })}
              className="h-7 text-xs font-mono"
            />
          </div>
          <div className="space-y-1">
            <Label className="text-xs">{t("common.description")}</Label>
            <Input
              value={item.description}
              placeholder={t("common.description")}
              onChange={(e) =>
                onUpdate({ ...item, description: e.target.value })
              }
              className="h-7 text-xs"
            />
          </div>
        </div>
        <div className="grid grid-cols-2 gap-2">
          <div className="space-y-1">
            <Label className="text-xs">
              {t("wizard.review.sourceTypeLabel")}
            </Label>
            <Select
              value={item.source_type ?? NONE_TYPE_SENTINEL}
              onValueChange={(v) =>
                onUpdate({
                  ...item,
                  source_type: v === NONE_TYPE_SENTINEL ? undefined : v,
                })
              }
            >
              <SelectTrigger className="h-7 text-xs">
                <SelectValue
                  placeholder={t("wizard.review.sourceTypeLabel")}
                />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NONE_TYPE_SENTINEL}>
                  {t("common.none")}
                </SelectItem>
                {entityTypeNames.map((name) => (
                  <SelectItem key={name} value={name}>
                    {name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1">
            <Label className="text-xs">
              {t("wizard.review.targetTypeLabel")}
            </Label>
            <Select
              value={item.target_type ?? NONE_TYPE_SENTINEL}
              onValueChange={(v) =>
                onUpdate({
                  ...item,
                  target_type: v === NONE_TYPE_SENTINEL ? undefined : v,
                })
              }
            >
              <SelectTrigger className="h-7 text-xs">
                <SelectValue
                  placeholder={t("wizard.review.targetTypeLabel")}
                />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NONE_TYPE_SENTINEL}>
                  {t("common.none")}
                </SelectItem>
                {entityTypeNames.map((name) => (
                  <SelectItem key={name} value={name}>
                    {name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
      </div>
      <Button
        variant="ghost"
        size="icon"
        className="h-7 w-7 mt-4 text-destructive hover:text-destructive"
        aria-label={t("wizard.review.deleteAriaLabel", { name: item.name })}
        onClick={() => setDeleteOpen(true)}
      >
        <Trash2 className="h-3 w-3" />
      </Button>
      <DeleteConfirmDialog
        open={deleteOpen}
        name={item.name}
        onConfirm={() => {
          setDeleteOpen(false);
          onDelete();
        }}
        onCancel={() => setDeleteOpen(false)}
      />
    </div>
  );
}

// ============================================================================
// Main component
// ============================================================================

interface ReviewStepProps {
  namespace: string;
  onNext: () => void;
  onBack: () => void;
}

export function ReviewStep({ namespace, onNext, onBack }: ReviewStepProps) {
  const { t } = useTranslation();
  const { schema: storeSchema, setSchema } = useWizardStore();

  // Load schema from store (set by ProposeStep) or fall back to server
  const { data: serverSchema, isLoading } = useQuery({
    queryKey: ["namespaceSchema", namespace],
    queryFn: () => getNamespaceSchema(namespace),
    enabled: !!namespace && !storeSchema,
    staleTime: 30_000,
  });

  // Resolve the proposal to display (store takes precedence over server fetch)
  const baseSchema = storeSchema ?? serverSchema ?? null;

  // Local editable state
  const [entityTypes, setEntityTypes] = useState<EntityTypeProposal[]>(
    () => baseSchema?.entity_types ?? [],
  );
  const [relationTypes, setRelationTypes] = useState<RelationTypeProposal[]>(
    () =>
      // Display editable relation types (already excludes RELATED_TO from server,
      // but filter again defensively)
      filterRelatedTo(baseSchema?.relation_types ?? []),
  );

  // Initialise editable lists when the schema first arrives
  const [initialised, setInitialised] = useState(false);
  if (baseSchema && !initialised) {
    setEntityTypes(baseSchema.entity_types ?? []);
    setRelationTypes(filterRelatedTo(baseSchema.relation_types ?? []));
    setInitialised(true);
  }

  const entityTypeNames = entityTypes.map((e) => e.name).filter(Boolean);

  // Save + proceed mutation
  const saveMutation = useMutation({
    mutationFn: async () => {
      // T-23-04: filter RELATED_TO before sending to server (defense-in-depth)
      const filteredRelationTypes = filterRelatedTo(relationTypes);
      // Pass-through without edits must NOT re-PATCH: a PATCH resets an
      // APPROVED schema back to "proposed", which breaks the out-of-band
      // preview/ingestion worker (it requires an approved schema). Only
      // write when the editable rows actually differ from the loaded schema.
      if (
        baseSchema &&
        JSON.stringify(entityTypes) ===
          JSON.stringify(baseSchema.entity_types ?? []) &&
        JSON.stringify(filteredRelationTypes) ===
          JSON.stringify(filterRelatedTo(baseSchema.relation_types ?? []))
      ) {
        return null; // unchanged — skip the PATCH, keep server status intact
      }
      const updated = await updateNamespaceSchema(namespace, {
        entity_types: entityTypes,
        relation_types: filteredRelationTypes,
      });
      return updated;
    },
    onSuccess: (updated) => {
      // Write filtered schema into wizard store for Preview step.
      // `updated` is the UNWRAPPED proposal (the API client strips the
      // { namespace, schema } envelope) — build the store schema from it
      // alone; never spread the raw response (CR-01).
      // `updated === null` ⇔ unchanged pass-through: carry the loaded schema
      // (preserving its server status, e.g. "approved") into the store.
      if (updated) {
        setSchema({
          ...updated,
          entity_types: updated.entity_types ?? [],
          relation_types: filterRelatedTo(updated.relation_types ?? []),
        });
      } else if (baseSchema) {
        setSchema({
          ...baseSchema,
          entity_types: baseSchema.entity_types ?? [],
          relation_types: filterRelatedTo(baseSchema.relation_types ?? []),
        });
      }
      onNext();
    },
    onError: (error) => {
      toast.error(t("common.saveFailed"), {
        description:
          error instanceof Error ? error.message : t("common.unknownError"),
      });
    },
  });

  // ── Loading state ──────────────────────────────────────────────────────────
  if (isLoading) {
    return (
      <Card>
        <CardContent className="flex items-center gap-2 py-8">
          <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
          <span className="text-sm text-muted-foreground">
            {t("common.loading")}
          </span>
        </CardContent>
      </Card>
    );
  }

  // ── Main render ────────────────────────────────────────────────────────────
  return (
    <TooltipProvider>
      <Card>
        <CardHeader>
          <CardTitle>{t("wizard.review.heading")}</CardTitle>
          <CardDescription>{t("wizard.review.description")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          {/* ── Entity types section ──────────────────────────────────────── */}
          <section className="space-y-3">
            <h3 className="text-sm font-medium">
              {t("wizard.review.entityTypesTitle")}
            </h3>
            <div className="space-y-2">
              {entityTypes.map((entity, idx) => (
                <EntityRow
                  key={idx}
                  item={entity}
                  onUpdate={(updated) =>
                    setEntityTypes((prev) =>
                      prev.map((e, i) => (i === idx ? updated : e)),
                    )
                  }
                  onDelete={() =>
                    setEntityTypes((prev) => prev.filter((_, i) => i !== idx))
                  }
                />
              ))}
            </div>
            <Button
              variant="outline"
              size="sm"
              className="gap-1.5"
              onClick={() =>
                setEntityTypes((prev) => [
                  ...prev,
                  {
                    name: "",
                    description: "",
                    frequency: 0,
                    is_baseline: false,
                  },
                ])
              }
            >
              <Plus className="h-3.5 w-3.5" />
              {t("wizard.review.addEntityType")}
            </Button>
          </section>

          {/* ── Relation types section ────────────────────────────────────── */}
          <section className="space-y-3">
            <h3 className="text-sm font-medium">
              {t("wizard.review.relationTypesTitle")}
            </h3>
            <div className="space-y-2">
              {relationTypes.map((relation, idx) => (
                <RelationRow
                  key={idx}
                  item={relation}
                  entityTypeNames={entityTypeNames}
                  onUpdate={(updated) =>
                    setRelationTypes((prev) =>
                      prev.map((r, i) => (i === idx ? updated : r)),
                    )
                  }
                  onDelete={() =>
                    setRelationTypes((prev) =>
                      prev.filter((_, i) => i !== idx),
                    )
                  }
                />
              ))}
            </div>
            <Button
              variant="outline"
              size="sm"
              className="gap-1.5"
              onClick={() =>
                setRelationTypes((prev) => [
                  ...prev,
                  { name: "", description: "", frequency: 0 },
                ])
              }
            >
              <Plus className="h-3.5 w-3.5" />
              {t("wizard.review.addRelationType")}
            </Button>

            {/* Fixed RELATED_TO row — rendered OUTSIDE the editable array (D-15 / PATTERNS.md 354-361) */}
            <div
              className="flex items-center gap-2 p-2 rounded border border-dashed border-muted-foreground/40 bg-muted/30"
              data-testid="related-to-fixed-row"
            >
              <Tooltip>
                <TooltipTrigger asChild>
                  <Lock className="h-3.5 w-3.5 text-muted-foreground flex-shrink-0 cursor-help" />
                </TooltipTrigger>
                <TooltipContent>
                  {t("wizard.review.relatedToDescription")}
                </TooltipContent>
              </Tooltip>
              <Badge variant="secondary" className="text-xs">
                {RELATED_TO}
              </Badge>
              <span className="text-xs text-muted-foreground flex-1">
                {t("wizard.review.relatedToDescription")}
              </span>
              {/* No delete button — RELATED_TO is non-removable */}
            </div>
          </section>

          {/* ── Navigation ────────────────────────────────────────────────── */}
          <div className="flex gap-2">
            <Button variant="outline" onClick={onBack}>
              {t("common.back")}
            </Button>
            <Button
              onClick={() => saveMutation.mutate()}
              disabled={saveMutation.isPending}
            >
              {saveMutation.isPending ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin mr-2" />
                  {t("common.saving")}
                </>
              ) : (
                t("wizard.review.cta")
              )}
            </Button>
          </div>
        </CardContent>
      </Card>
    </TooltipProvider>
  );
}
