/**
 * @module ReviewStep — STUB (Plan 06)
 * @description Placeholder for the schema review editor step (D-15).
 *
 * This is a STUB created in Plan 06 so the ingestion-wizard.tsx shell can import
 * ReviewStep and next build passes. Plan 07 replaces this file with the real
 * entity/relation type editor.
 *
 * Props match the final interface that Plan 07 expects.
 */

"use client";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useTranslation } from "react-i18next";

interface ReviewStepProps {
  namespace: string;
  onNext: () => void;
  onBack: () => void;
}

/**
 * STUB: Plan 07 replaces this with the real schema editor (D-15).
 */
export function ReviewStep({ onNext, onBack }: ReviewStepProps) {
  const { t } = useTranslation();

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t("wizard.steps.review")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">
          {t("wizard.review.description")}
        </p>
        <div className="flex gap-2">
          <Button variant="outline" onClick={onBack}>
            {t("common.back")}
          </Button>
          <Button onClick={onNext}>
            {t("wizard.review.cta")}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}
