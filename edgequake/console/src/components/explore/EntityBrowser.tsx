import { useEffect, useMemo, useRef, useState } from 'react';
import { Search } from 'lucide-react';
import { Input } from '@/components/ui/input';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { usePipelineConfig } from '@/hooks/usePipelineConfig';
import {
  useEntityList,
  type EntityItem,
} from '@/hooks/useEntities';

// ---------------------------------------------------------------------------
// Type badge colors — deterministic per entity type via char-code hash
// ---------------------------------------------------------------------------

const TYPE_COLORS = [
  'bg-blue-100 text-blue-800 dark:bg-blue-900/40 dark:text-blue-300',
  'bg-green-100 text-green-800 dark:bg-green-900/40 dark:text-green-300',
  'bg-purple-100 text-purple-800 dark:bg-purple-900/40 dark:text-purple-300',
  'bg-orange-100 text-orange-800 dark:bg-orange-900/40 dark:text-orange-300',
  'bg-pink-100 text-pink-800 dark:bg-pink-900/40 dark:text-pink-300',
  'bg-yellow-100 text-yellow-800 dark:bg-yellow-900/40 dark:text-yellow-300',
  'bg-teal-100 text-teal-800 dark:bg-teal-900/40 dark:text-teal-300',
  'bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300',
] as const;

function typeColor(entityType: string): string {
  let hash = 0;
  for (let i = 0; i < entityType.length; i++) {
    hash += entityType.charCodeAt(i);
  }
  return TYPE_COLORS[hash % TYPE_COLORS.length];
}

function truncate(text: string | null, max = 100): string {
  if (!text) return '';
  return text.length > max ? text.slice(0, max) + '...' : text;
}

// ---------------------------------------------------------------------------
// EntityBrowser
// ---------------------------------------------------------------------------

const ALL_TYPES_VALUE = '__all__';

interface EntityBrowserProps {
  slug: string;
}

export function EntityBrowser({ slug }: EntityBrowserProps) {
  // ---- Filter / pagination state ----
  const [search, setSearch] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  const [entityType, setEntityType] = useState('');
  const [page, setPage] = useState(1);
  const [items, setItems] = useState<EntityItem[]>([]);

  // ---- Debounce search input (300ms) ----
  useEffect(() => {
    const id = setTimeout(() => setDebouncedSearch(search), 300);
    return () => clearTimeout(id);
  }, [search]);

  // ---- Data fetching ----
  const { data: pipelineConfig } = usePipelineConfig(slug);
  const entityTypes: string[] = useMemo(() => {
    const raw = pipelineConfig?.entity_types;
    if (Array.isArray(raw)) return raw as string[];
    if (typeof raw === 'string') {
      try {
        const parsed = JSON.parse(raw);
        if (Array.isArray(parsed)) return parsed as string[];
      } catch {
        /* ignore */
      }
    }
    return [];
  }, [pipelineConfig?.entity_types]);

  const {
    data,
    isFetching,
    isLoading,
    isError,
    error,
    refetch,
  } = useEntityList(slug, debouncedSearch, entityType, page);

  // ---- Reset when filters change ----
  const prevSearch = useRef(debouncedSearch);
  const prevType = useRef(entityType);

  useEffect(() => {
    if (
      prevSearch.current !== debouncedSearch ||
      prevType.current !== entityType
    ) {
      setItems([]);
      setPage(1);
      prevSearch.current = debouncedSearch;
      prevType.current = entityType;
    }
  }, [debouncedSearch, entityType]);

  // ---- Accumulate items when data arrives ----
  useEffect(() => {
    if (!data) return;
    if (data.page === 1) {
      setItems(data.items);
    } else {
      setItems((prev) => [...prev, ...data.items]);
    }
  }, [data]);

  // ---- Derived state ----
  const totalPages = data?.total_pages ?? 0;
  const hasMore = page < totalPages;
  const isFirstLoad = isLoading && items.length === 0;
  const filtersActive = debouncedSearch !== '' || entityType !== '';
  const isEmpty = !isLoading && !isFetching && items.length === 0;

  // ---- Render ----
  return (
    <div className="space-y-4">
      {/* Search + type filter row */}
      <div className="flex gap-3">
        <div className="relative flex-1">
          <Search className="absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            placeholder="Search entities..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="pl-8"
          />
        </div>
        <Select
          value={entityType || ALL_TYPES_VALUE}
          onValueChange={(v) => setEntityType(v === ALL_TYPES_VALUE ? '' : v)}
        >
          <SelectTrigger className="w-48">
            <SelectValue placeholder="All types" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ALL_TYPES_VALUE}>All types</SelectItem>
            {entityTypes.map((t) => (
              <SelectItem key={t} value={t}>
                {t}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {/* Error state */}
      {isError && (
        <div className="rounded-md border border-red-200 bg-red-50 p-4 dark:border-red-800 dark:bg-red-950">
          <p className="text-sm text-red-700 dark:text-red-400">
            Failed to load entities{error instanceof Error ? `: ${error.message}` : '.'}
          </p>
          <Button
            variant="outline"
            size="sm"
            className="mt-2"
            onClick={() => refetch()}
          >
            Retry
          </Button>
        </div>
      )}

      {/* Loading skeleton (initial load only) */}
      {isFirstLoad && !isError && (
        <div className="space-y-2">
          {Array.from({ length: 6 }).map((_, i) => (
            <Skeleton key={i} className="h-10 w-full" />
          ))}
        </div>
      )}

      {/* Empty: no entities at all (no filters) */}
      {isEmpty && !filtersActive && !isError && (
        <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-12 text-center">
          <p className="text-sm text-muted-foreground">
            No entities found in this namespace. Run the ingestion pipeline to
            extract entities.
          </p>
        </div>
      )}

      {/* Empty: no search results */}
      {isEmpty && filtersActive && !isError && (
        <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-8 text-center">
          <p className="text-sm text-muted-foreground">
            No entities match your search.
          </p>
        </div>
      )}

      {/* Entity table */}
      {items.length > 0 && (
        <>
          <div className="overflow-x-auto rounded-md border">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b bg-muted/50">
                  <th className="px-3 py-2 text-left font-medium">Name</th>
                  <th className="px-3 py-2 text-left font-medium">Type</th>
                  <th className="px-3 py-2 text-right font-medium">Degree</th>
                  <th className="px-3 py-2 text-left font-medium">
                    Description
                  </th>
                </tr>
              </thead>
              <tbody>
                {items.map((entity, idx) => (
                  <tr
                    key={`${entity.name}-${entity.entity_type}-${idx}`}
                    className="border-b last:border-b-0"
                  >
                    <td className="px-3 py-2 font-medium">{entity.name}</td>
                    <td className="px-3 py-2">
                      <Badge
                        variant="secondary"
                        className={typeColor(entity.entity_type)}
                      >
                        {entity.entity_type}
                      </Badge>
                    </td>
                    <td className="px-3 py-2 text-right tabular-nums">
                      {entity.degree}
                    </td>
                    <td className="max-w-xs px-3 py-2 text-muted-foreground">
                      {truncate(entity.description)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          {/* Load more */}
          {hasMore && (
            <div className="flex justify-center">
              <Button
                variant="outline"
                disabled={isFetching}
                onClick={() => setPage((p) => p + 1)}
              >
                {isFetching ? 'Loading...' : 'Load more'}
              </Button>
            </div>
          )}
        </>
      )}
    </div>
  );
}
