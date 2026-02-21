import { Plus } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Skeleton } from '@/components/ui/skeleton';
import { Card } from '@/components/ui/card';
import { NamespaceCard } from '@/components/namespace/NamespaceCard';
import { CreateNamespaceDialog } from '@/components/namespace/CreateNamespaceDialog';
import { useNamespaces } from '@/hooks/useNamespaces';

function SkeletonCard() {
  return (
    <Card className="flex flex-col gap-6 py-6">
      <div className="px-6">
        <div className="flex items-center justify-between">
          <Skeleton className="h-5 w-32" />
          <Skeleton className="h-5 w-16 rounded-full" />
        </div>
        <Skeleton className="mt-3 h-4 w-48" />
      </div>
      <div className="grid grid-cols-2 gap-3 px-6">
        <Skeleton className="h-4 w-24" />
        <Skeleton className="h-4 w-20" />
      </div>
      <div className="flex justify-between px-6">
        <Skeleton className="h-3 w-28" />
        <Skeleton className="h-3 w-24" />
      </div>
    </Card>
  );
}

export function DashboardPage() {
  const { data: namespaces, isLoading, error } = useNamespaces();

  return (
    <div className="p-6">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Namespaces</h1>
          <p className="text-sm text-muted-foreground">
            Manage your Graph RAG pipelines
          </p>
        </div>
        <CreateNamespaceDialog
          trigger={
            <Button>
              <Plus />
              Create Namespace
            </Button>
          }
        />
      </div>

      {error && (
        <div className="rounded-lg border border-destructive/50 bg-destructive/10 p-4 text-sm text-destructive">
          Failed to load namespaces: {error.message}
        </div>
      )}

      {isLoading && (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
          <SkeletonCard />
          <SkeletonCard />
          <SkeletonCard />
        </div>
      )}

      {!isLoading && !error && namespaces && namespaces.length === 0 && (
        <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-12 text-center">
          <h3 className="text-lg font-semibold">No namespaces yet</h3>
          <p className="mt-1 text-sm text-muted-foreground">
            Create your first namespace to start building a Graph RAG pipeline.
          </p>
          <CreateNamespaceDialog
            trigger={
              <Button className="mt-4">
                <Plus />
                Create Namespace
              </Button>
            }
          />
        </div>
      )}

      {!isLoading && !error && namespaces && namespaces.length > 0 && (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
          {namespaces.map((ns) =>
            ns ? (
              <NamespaceCard
                key={ns.slug}
                slug={ns.slug}
                description={ns.description}
                createdAt={ns.created_at}
              />
            ) : null
          )}
        </div>
      )}
    </div>
  );
}
