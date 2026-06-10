/**
 * @module IngestionWizardEntryButton
 * @description Outline button that navigates to the ingestion wizard route.
 *
 * Placement: workspace page action bar, to the LEFT of the rebuild buttons.
 * Visually distinct from the primary rebuild CTAs by `variant="outline"` + Wand2 icon
 * (Pitfall 4 — must not blend with or replace the rebuild CTAs).
 *
 * @implements D-14 — Guided ingestion wizard entry point
 */

"use client";

import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Wand2 } from "lucide-react";
import { useRouter } from "next/navigation";
import { useTranslation } from "react-i18next";

export function IngestionWizardEntryButton() {
  const { t } = useTranslation();
  const router = useRouter();

  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="outline"
            onClick={() => router.push("/workspace/wizard")}
            className="gap-2"
            data-testid="ingestion-wizard-entry-button"
          >
            <Wand2 className="h-4 w-4" />
            {t("wizard.entryButton")}
          </Button>
        </TooltipTrigger>
        <TooltipContent>
          <p>{t("wizard.entryButtonTooltip")}</p>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
