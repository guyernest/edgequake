import { AlertTriangle } from 'lucide-react';

interface PreviewCountsTableProps {
  title: string;
  counts: Array<{ typeName: string; count: number }>;
}

export function PreviewCountsTable({ title, counts }: PreviewCountsTableProps) {
  const sorted = [...counts].sort((a, b) => b.count - a.count);
  const total = sorted.reduce((sum, row) => sum + row.count, 0);

  return (
    <div>
      <h4 className="mb-2 text-sm font-semibold">{title}</h4>
      <div className="rounded-md border">
        <table className="w-full">
          <thead>
            <tr className="border-b bg-muted/50">
              <th className="p-2 text-left text-xs font-medium text-muted-foreground">
                Type Name
              </th>
              <th className="w-24 p-2 text-right text-xs font-medium text-muted-foreground">
                Count
              </th>
            </tr>
          </thead>
          <tbody>
            {sorted.map((row) => (
              <tr
                key={row.typeName}
                className={
                  row.count === 0
                    ? 'border-b bg-amber-50 dark:bg-amber-950/30'
                    : 'border-b hover:bg-muted/50'
                }
              >
                <td className="p-2 text-sm font-medium">
                  <span className="flex items-center gap-1.5">
                    {row.count === 0 && (
                      <AlertTriangle className="size-3.5 shrink-0 text-amber-500" />
                    )}
                    {row.typeName}
                  </span>
                </td>
                <td className="p-2 text-right text-sm tabular-nums">
                  {row.count}
                </td>
              </tr>
            ))}
            {sorted.length === 0 && (
              <tr>
                <td
                  colSpan={2}
                  className="p-4 text-center text-sm text-muted-foreground"
                >
                  No types to display
                </td>
              </tr>
            )}
            {sorted.length > 0 && (
              <tr className="bg-muted/30">
                <td
                  colSpan={2}
                  className="p-2 text-right text-sm font-semibold tabular-nums"
                >
                  Total: {total}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
