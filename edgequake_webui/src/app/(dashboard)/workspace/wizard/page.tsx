/**
 * @module WizardPage
 * @description Ingestion wizard route — /workspace/wizard (D-14 Phase 23).
 *
 * Resolves the namespace slug from the currently selected workspace using
 * resolveNamespaceSlug (Wave-0 CASE A — Plan 03). If the namespace cannot be
 * resolved, renders a friendly fallback instead of mounting the wizard (T-23-07
 * guard: prevents /namespaces/undefined/... API calls).
 *
 * @route /workspace/wizard
 */

"use client";

import { IngestionWizard } from "@/components/workspace/ingestion-wizard/ingestion-wizard";
import { Card, CardContent } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { NamespaceSlugError, resolveNamespaceSlug } from "@/lib/namespace-resolve";
import { useTenantStore } from "@/stores/use-tenant-store";
import { getWorkspace } from "@/lib/api/edgequake";
import { useQuery } from "@tanstack/react-query";
import { FolderKanban } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Skeleton } from "@/components/ui/skeleton";

export default function WizardPage() {
  const { t } = useTranslation();
  const { selectedTenantId, selectedWorkspaceId } = useTenantStore();

  // Fetch the workspace to resolve namespace_slug (Wave-0 CASE A: Plan 01 confirmed
  // workspace.namespace_slug = workspace.slug — no runtime probe needed).
  const { data: workspace, isLoading } = useQuery({
    queryKey: ["workspace", selectedTenantId, selectedWorkspaceId],
    queryFn: () =>
      selectedTenantId && selectedWorkspaceId
        ? getWorkspace(selectedTenantId, selectedWorkspaceId)
        : Promise.reject(new Error("No workspace selected")),
    enabled: !!selectedTenantId && !!selectedWorkspaceId,
    staleTime: 30000,
  });

  // Guard: no workspace selected at all
  if (!selectedTenantId || !selectedWorkspaceId) {
    return (
      <div className="container mx-auto p-6">
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <FolderKanban className="h-12 w-12 text-muted-foreground mb-4" />
            <h2 className="text-lg font-medium text-muted-foreground">
              {t("workspace.noWorkspaceSelected", "No Workspace Selected")}
            </h2>
            <p className="text-sm text-muted-foreground mt-2">
              {t(
                "workspace.selectWorkspaceHint",
                "Please select a workspace from the sidebar.",
              )}
            </p>
          </CardContent>
        </Card>
      </div>
    );
  }

  // Loading state
  if (isLoading || !workspace) {
    return (
      <div className="container mx-auto p-6 space-y-6">
        <Skeleton className="h-8 w-48" />
        <Skeleton className="h-12 w-full" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  // Resolve namespace slug (T-23-07 guard — prevent /namespaces/undefined/... calls)
  let resolvedNamespace: string | null = null;
  try {
    resolvedNamespace = resolveNamespaceSlug(workspace);
  } catch (e) {
    if (!(e instanceof NamespaceSlugError)) throw e;
    // Workspace lacks a namespace_slug — show a friendly fallback.
  }

  if (!resolvedNamespace) {
    return (
      <div className="container mx-auto p-6">
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <FolderKanban className="h-12 w-12 text-muted-foreground mb-4" />
            <h2 className="text-lg font-medium text-muted-foreground">
              {t("workspace.notFound", "Workspace configuration unavailable")}
            </h2>
            <p className="text-sm text-muted-foreground mt-2">
              {t(
                "workspace.selectWorkspaceHint",
                "This workspace does not have a namespace configured. Please contact support.",
              )}
            </p>
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    // The dashboard layout's <main> is overflow-hidden — each page owns its
    // scrolling. Same ScrollArea pattern as workspace/page.tsx; without it the
    // wizard's Review step is clipped with no way to reach the bottom buttons.
    <ScrollArea className="h-[calc(100vh-theme(spacing.20))]">
      <div className="container mx-auto p-6">
        <IngestionWizard
          namespace={resolvedNamespace}
          workspaceId={selectedWorkspaceId}
        />
      </div>
    </ScrollArea>
  );
}
