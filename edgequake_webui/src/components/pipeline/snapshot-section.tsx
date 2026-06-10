/**
 * @module SnapshotSection
 * @description Run-report card for snapshot export results (D-09/D-10 — Phase 23 Plan 05).
 *
 * Only rendered when the ingestion run produced a snapshot (snapshot field is defined
 * on the IngestionResult). Shows the destination URI, exported-at time, and all 6
 * SnapshotCounts (documents, chunks, entities, relationships, vectors, bm25).
 *
 * @implements D-09 — Snapshot section absent when run produced no snapshot
 * @implements D-10 — Shows all 6 counts + URI + exported-at in the run report
 *
 * @security T-23-08: URI rendered inside a <code> element — React escapes by default,
 * no dangerouslySetInnerHTML. Counts coerced via toLocaleString() on numbers only.
 * T-23-09: Returns null when snapshot is undefined — malformed/absent event safe.
 */
'use client';

import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import type { IngestionResult } from '@/types/ingestion';
import { formatDistanceToNow } from 'date-fns';
import { PackageOpen } from 'lucide-react';
import { useTranslation } from 'react-i18next';

export interface SnapshotSectionProps {
  /** Snapshot export result from the completed ingestion run. Returns null when undefined (D-09). */
  snapshot?: IngestionResult['snapshot'];
}

/**
 * Snapshot run-report card.
 *
 * Returns null when `snapshot` is undefined (D-09 — only render when run produced a snapshot).
 */
export function SnapshotSection({ snapshot }: SnapshotSectionProps) {
  const { t } = useTranslation();

  // D-09: absent when no snapshot (T-23-09: safe against malformed/absent event)
  if (!snapshot) return null;

  const exportedAgo = (() => {
    try {
      return formatDistanceToNow(new Date(snapshot.created_at), { addSuffix: true });
    } catch {
      return snapshot.created_at;
    }
  })();

  // The 6 count keys defined in IngestionResult['snapshot']['counts']
  const countKeys = [
    'documents',
    'chunks',
    'entities',
    'relationships',
    'vectors',
    'bm25',
  ] as const;

  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-lg flex items-center gap-2">
          <PackageOpen className="h-5 w-5" />
          {t('pipeline.snapshot.title')}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        {/* Destination URI — rendered as text inside <code>, React escapes (T-23-08) */}
        <div>
          <p className="text-xs text-muted-foreground mb-1">
            {t('pipeline.snapshot.destinationLabel')}
          </p>
          <code className="block text-xs font-mono break-all bg-muted/50 px-2 py-1 rounded">
            {snapshot.uri}
          </code>
        </div>

        {/* Exported-at time */}
        <div>
          <p className="text-xs text-muted-foreground">
            {t('pipeline.snapshot.exportedAtLabel')}{' '}
            <span className="text-foreground">{exportedAgo}</span>
          </p>
        </div>

        {/* 6-count grid (D-10) */}
        <div>
          <p className="text-xs text-muted-foreground mb-2">
            {t('pipeline.snapshot.countsTableHeader')}
          </p>
          <div className="grid grid-cols-3 gap-3">
            {countKeys.map((key) => (
              <div key={key} className="text-center">
                <div className="text-lg font-semibold">
                  {snapshot.counts[key].toLocaleString()}
                </div>
                <div className="text-xs text-muted-foreground">
                  {t(`pipeline.snapshot.counts.${key}`)}
                </div>
              </div>
            ))}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

export default SnapshotSection;
