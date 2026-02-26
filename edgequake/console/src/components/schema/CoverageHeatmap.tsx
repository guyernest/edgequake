import * as Tooltip from '@radix-ui/react-tooltip';

interface CoverageHeatmapProps {
  coverageRows: Array<{ entityType: string; counts: number[] }>;
  documentColumns: Array<{ id: string; name: string; truncatedName: string }>;
}

function cellColor(count: number): string {
  if (count === 0) return 'bg-red-100 dark:bg-red-950';
  if (count <= 2) return 'bg-yellow-100 dark:bg-yellow-950';
  return 'bg-green-100 dark:bg-green-950';
}

function isFullZeroRow(counts: number[]): boolean {
  return counts.every((c) => c === 0);
}

export function CoverageHeatmap({
  coverageRows,
  documentColumns,
}: CoverageHeatmapProps) {
  if (coverageRows.length === 0 || documentColumns.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No coverage data to display.
      </p>
    );
  }

  return (
    <Tooltip.Provider delayDuration={200}>
      <div className="overflow-x-auto rounded-md border">
        <table className="w-full border-collapse">
          <thead>
            <tr className="border-b bg-muted/50">
              <th className="sticky left-0 z-10 bg-muted/50 p-2 text-left text-xs font-medium text-muted-foreground">
                Entity Type
              </th>
              {documentColumns.map((doc) => (
                <Tooltip.Root key={doc.id}>
                  <Tooltip.Trigger asChild>
                    <th className="min-w-[80px] p-2 text-center text-xs font-medium text-muted-foreground">
                      <span className="cursor-default truncate">
                        {doc.truncatedName}
                      </span>
                    </th>
                  </Tooltip.Trigger>
                  <Tooltip.Portal>
                    <Tooltip.Content
                      className="rounded-md bg-popover px-3 py-1.5 text-xs text-popover-foreground shadow-md border"
                      sideOffset={5}
                    >
                      {doc.name}
                      <Tooltip.Arrow className="fill-popover" />
                    </Tooltip.Content>
                  </Tooltip.Portal>
                </Tooltip.Root>
              ))}
            </tr>
          </thead>
          <tbody>
            {coverageRows.map((row) => {
              const fullZero = isFullZeroRow(row.counts);
              return (
                <tr
                  key={row.entityType}
                  className={
                    fullZero
                      ? 'border-b bg-amber-50/50 dark:bg-amber-950/20'
                      : 'border-b'
                  }
                >
                  <td className="sticky left-0 z-10 bg-background p-2 text-sm font-medium">
                    {fullZero && (
                      <span className="mr-1 inline-block size-2 rounded-full bg-amber-400" />
                    )}
                    {row.entityType}
                  </td>
                  {row.counts.map((count, colIdx) => {
                    const doc = documentColumns[colIdx];
                    return (
                      <Tooltip.Root key={`${row.entityType}-${doc?.id ?? colIdx}`}>
                        <Tooltip.Trigger asChild>
                          <td
                            className={`cursor-default p-2 text-center text-xs tabular-nums ${cellColor(count)}`}
                          >
                            {count}
                          </td>
                        </Tooltip.Trigger>
                        <Tooltip.Portal>
                          <Tooltip.Content
                            className="rounded-md bg-popover px-3 py-1.5 text-xs text-popover-foreground shadow-md border"
                            sideOffset={5}
                          >
                            {doc?.name ?? `Document ${colIdx + 1}`}: {count}{' '}
                            {row.entityType} entities found
                            <Tooltip.Arrow className="fill-popover" />
                          </Tooltip.Content>
                        </Tooltip.Portal>
                      </Tooltip.Root>
                    );
                  })}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </Tooltip.Provider>
  );
}
