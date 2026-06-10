/**
 * @module PreviewStep — STUB (Plan 06)
 * @description Placeholder for the extraction preview step (D-16).
 *
 * This is a STUB created in Plan 06 so the ingestion-wizard.tsx shell can import
 * PreviewStep and next build passes. Plan 07 replaces this file with the real
 * preview tabs (chunks/entities/relations aggregate view).
 *
 * Props match the final interface that Plan 07 expects.
 */

"use client";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useTranslation } from "react-i18next";

interface PreviewStepProps {
  namespace: string;
  onNext: () => void;
  onBack: () => void;
}

/**
 * STUB: Plan 07 replaces this with the real preview tabs (D-16).
 */
export function PreviewStep({ onNext, onBack }: PreviewStepProps) {
  const { t } = useTranslation();

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("wizard.steps.preview")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">
          {t("wizard.preview.description")}
        </p>
        <div className="flex gap-2">
          <Button variant="outline" onClick={onBack}>
            {t("common.back")}
          </Button>
          <Button onClick={onNext}>
            {t("wizard.preview.cta")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
