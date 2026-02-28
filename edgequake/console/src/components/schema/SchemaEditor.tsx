import { useState, useCallback, useEffect } from 'react';
import { Check, X, Plus, Trash2, Pencil, Eye } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Badge } from '@/components/ui/badge';
import { Separator } from '@/components/ui/separator';
import { Skeleton } from '@/components/ui/skeleton';
import { useSchemaProposal } from '@/hooks/useNamespaceDetail';
import {
  useApproveSchema,
  useRejectSchema,
  useUpdateSchemaTypes,
} from '@/hooks/useSchemaActions';
import { usePreviewExtraction } from '@/hooks/usePreviewExtraction';
import { PreviewConfirmModal } from './PreviewConfirmModal';
import { PreviewResultsPanel, type PreviewResult } from './PreviewResultsPanel';

interface EntityType {
  name: string;
  description: string;
  frequency: number;
  is_baseline: boolean;
}

interface RelationType {
  name: string;
  description: string;
  source_type: string;
  target_type: string;
  frequency: number;
}

function parseJsonField<T>(val: unknown): T[] {
  if (Array.isArray(val)) return val as T[];
  if (typeof val === 'string') {
    try { return JSON.parse(val) as T[]; } catch { return []; }
  }
  return [];
}

function EntityRow({
  entity,
  isEditable,
  onRemove,
  onUpdate,
  previewCount,
}: {
  entity: EntityType;
  isEditable: boolean;
  onRemove: () => void;
  onUpdate: (updated: EntityType) => void;
  previewCount?: number;
}) {
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(entity.name);
  const [description, setDescription] = useState(entity.description);

  function save() {
    onUpdate({ ...entity, name: name.trim(), description: description.trim() });
    setEditing(false);
  }

  function cancel() {
    setName(entity.name);
    setDescription(entity.description);
    setEditing(false);
  }

  if (editing) {
    return (
      <tr className="border-b">
        <td className="p-2">
          <Input value={name} onChange={(e) => setName(e.target.value)} className="h-8 text-sm" />
        </td>
        <td className="p-2">
          <Input value={description} onChange={(e) => setDescription(e.target.value)} className="h-8 text-sm" />
        </td>
        <td className="p-2 text-center">
          {entity.is_baseline ? <Badge variant="secondary">baseline</Badge> : null}
        </td>
        <td className="p-2 text-right">
          <div className="flex justify-end gap-1">
            <Button variant="ghost" size="sm" onClick={save} className="h-7 w-7 p-0">
              <Check className="size-3.5" />
            </Button>
            <Button variant="ghost" size="sm" onClick={cancel} className="h-7 w-7 p-0">
              <X className="size-3.5" />
            </Button>
          </div>
        </td>
      </tr>
    );
  }

  return (
    <tr className="border-b hover:bg-muted/50">
      <td className="p-2 text-sm font-medium">
        <span className="flex items-center gap-1.5">
          {entity.name}
          {previewCount === 0 && (
            <span className="inline-flex items-center rounded-full border border-amber-300 bg-amber-50 px-1.5 py-0.5 text-[10px] font-medium text-amber-700 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-400">
              0 hits
            </span>
          )}
          {previewCount != null && previewCount > 0 && (
            <span className="text-[10px] text-muted-foreground">{previewCount}</span>
          )}
        </span>
      </td>
      <td className="p-2 text-sm text-muted-foreground">{entity.description}</td>
      <td className="p-2 text-center">
        {entity.is_baseline ? <Badge variant="secondary">baseline</Badge> : null}
      </td>
      <td className="p-2 text-right">
        {isEditable && (
          <div className="flex justify-end gap-1">
            <Button variant="ghost" size="sm" onClick={() => setEditing(true)} className="h-7 w-7 p-0">
              <Pencil className="size-3.5" />
            </Button>
            <Button variant="ghost" size="sm" onClick={onRemove} className="h-7 w-7 p-0 text-destructive hover:text-destructive">
              <Trash2 className="size-3.5" />
            </Button>
          </div>
        )}
      </td>
    </tr>
  );
}

function RelationRow({
  relation,
  isEditable,
  onRemove,
  onUpdate,
  previewCount,
}: {
  relation: RelationType;
  isEditable: boolean;
  onRemove: () => void;
  onUpdate: (updated: RelationType) => void;
  previewCount?: number;
}) {
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(relation.name);
  const [description, setDescription] = useState(relation.description);
  const [sourceType, setSourceType] = useState(relation.source_type);
  const [targetType, setTargetType] = useState(relation.target_type);

  function save() {
    onUpdate({
      ...relation,
      name: name.trim(),
      description: description.trim(),
      source_type: sourceType.trim(),
      target_type: targetType.trim(),
    });
    setEditing(false);
  }

  function cancel() {
    setName(relation.name);
    setDescription(relation.description);
    setSourceType(relation.source_type);
    setTargetType(relation.target_type);
    setEditing(false);
  }

  if (editing) {
    return (
      <tr className="border-b">
        <td className="p-2">
          <Input value={name} onChange={(e) => setName(e.target.value)} className="h-8 text-sm" />
        </td>
        <td className="p-2">
          <Input value={description} onChange={(e) => setDescription(e.target.value)} className="h-8 text-sm" />
        </td>
        <td className="p-2">
          <div className="flex items-center gap-1">
            <Input value={sourceType} onChange={(e) => setSourceType(e.target.value)} className="h-8 w-28 text-xs" />
            <span className="text-muted-foreground">→</span>
            <Input value={targetType} onChange={(e) => setTargetType(e.target.value)} className="h-8 w-28 text-xs" />
          </div>
        </td>
        <td className="p-2 text-right">
          <div className="flex justify-end gap-1">
            <Button variant="ghost" size="sm" onClick={save} className="h-7 w-7 p-0">
              <Check className="size-3.5" />
            </Button>
            <Button variant="ghost" size="sm" onClick={cancel} className="h-7 w-7 p-0">
              <X className="size-3.5" />
            </Button>
          </div>
        </td>
      </tr>
    );
  }

  return (
    <tr className="border-b hover:bg-muted/50">
      <td className="p-2 text-sm font-medium">
        <span className="flex items-center gap-1.5">
          {relation.name}
          {previewCount === 0 && (
            <span className="inline-flex items-center rounded-full border border-amber-300 bg-amber-50 px-1.5 py-0.5 text-[10px] font-medium text-amber-700 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-400">
              0 hits
            </span>
          )}
          {previewCount != null && previewCount > 0 && (
            <span className="text-[10px] text-muted-foreground">{previewCount}</span>
          )}
        </span>
      </td>
      <td className="p-2 text-sm text-muted-foreground">{relation.description}</td>
      <td className="p-2 text-sm">
        <span className="font-mono text-xs">{relation.source_type}</span>
        <span className="mx-1 text-muted-foreground">→</span>
        <span className="font-mono text-xs">{relation.target_type}</span>
      </td>
      <td className="p-2 text-right">
        {isEditable && (
          <div className="flex justify-end gap-1">
            <Button variant="ghost" size="sm" onClick={() => setEditing(true)} className="h-7 w-7 p-0">
              <Pencil className="size-3.5" />
            </Button>
            <Button variant="ghost" size="sm" onClick={onRemove} className="h-7 w-7 p-0 text-destructive hover:text-destructive">
              <Trash2 className="size-3.5" />
            </Button>
          </div>
        )}
      </td>
    </tr>
  );
}

export function SchemaEditor({ slug }: { slug: string }) {
  const { data: schema, isLoading } = useSchemaProposal(slug);
  const approveMutation = useApproveSchema(slug);
  const rejectMutation = useRejectSchema(slug);
  const updateMutation = useUpdateSchemaTypes(slug);

  const [entities, setEntities] = useState<EntityType[] | null>(null);
  const [relations, setRelations] = useState<RelationType[] | null>(null);
  const [dirty, setDirty] = useState(false);
  const [editingApproved, setEditingApproved] = useState(false);
  const [showPreviewModal, setShowPreviewModal] = useState(false);
  const [showResultsPanel, setShowResultsPanel] = useState(false);
  const [finalPreviewResult, setFinalPreviewResult] = useState<PreviewResult | null>(null);
  const [schemaSavedAfterPreview, setSchemaSavedAfterPreview] = useState(false);

  const {
    fetchCostEstimate,
    costEstimate,
    isCostLoading,
    costError,
    triggerPreview,
    triggerStatus,
    isTriggerPending,
    triggerError,
    previewResult,
    isPreviewRunning,
    documentsCompleted,
    documentsTotal,
    cancelPolling,
    resetPreview,
  } = usePreviewExtraction(slug);

  // When preview completes (or is cancelled with partial results), store the result and open panel
  useEffect(() => {
    const status = previewResult?.status;
    if (status === 'completed' || status === 'cancelled') {
      setFinalPreviewResult(previewResult as PreviewResult);
      setShowPreviewModal(false);
      setShowResultsPanel(true);
      setSchemaSavedAfterPreview(false); // Fresh results, not stale
    }
  }, [previewResult?.status, previewResult]);

  // Parse and initialize local state from schema data
  const entityTypes: EntityType[] = parseJsonField(schema?.entity_types);
  const relationTypes: RelationType[] = parseJsonField(schema?.relation_types);

  const currentEntities = entities ?? entityTypes;
  const currentRelations = relations ?? relationTypes;

  // Build preview count lookups from latest preview result
  const entityCountMap = new Map<string, number>();
  const relationCountMap = new Map<string, number>();
  if (finalPreviewResult) {
    (finalPreviewResult.entityTypeCounts ?? []).forEach((c) => entityCountMap.set(c.typeName, c.count));
    (finalPreviewResult.relationTypeCounts ?? []).forEach((c) => relationCountMap.set(c.typeName, c.count));
  }

  const isProposed = schema?.status === 'proposed';
  const isApproved = schema?.status === 'approved';
  const isEditable = isProposed || editingApproved;

  const updateEntities = useCallback((updated: EntityType[]) => {
    setEntities(updated);
    setDirty(true);
  }, []);

  const updateRelations = useCallback((updated: RelationType[]) => {
    setRelations(updated);
    setDirty(true);
  }, []);

  function handleAddEntity() {
    updateEntities([
      ...currentEntities,
      { name: 'NEW_TYPE', description: 'Description', frequency: 0, is_baseline: false },
    ]);
  }

  function handleAddRelation() {
    updateRelations([
      ...currentRelations,
      { name: 'new_relation', description: 'Description', source_type: 'PERSON', target_type: 'ORGANIZATION', frequency: 0 },
    ]);
  }

  function handleSave() {
    updateMutation.mutate(
      { entity_types: currentEntities, relation_types: currentRelations },
      {
        onSuccess: () => {
          setEntities(null);
          setRelations(null);
          setDirty(false);
          setEditingApproved(false);
          // Mark existing preview results as stale after schema change
          if (finalPreviewResult) {
            setSchemaSavedAfterPreview(true);
          }
        },
      }
    );
  }

  function handleApprove() {
    approveMutation.mutate(undefined, {
      onSuccess: () => {
        setEntities(null);
        setRelations(null);
        setDirty(false);
        setEditingApproved(false);
      },
    });
  }

  function handleCancelEdit() {
    setEntities(null);
    setRelations(null);
    setDirty(false);
    setEditingApproved(false);
  }

  if (isLoading) {
    return (
      <div className="space-y-3 p-4">
        <Skeleton className="h-5 w-40" />
        <Skeleton className="h-4 w-64" />
        <Skeleton className="h-4 w-48" />
      </div>
    );
  }

  if (!schema) {
    return (
      <div className="flex flex-col items-center justify-center rounded-lg border border-dashed p-12 text-center">
        <h3 className="text-base font-semibold">No schema proposal</h3>
        <p className="mt-1 text-sm text-muted-foreground">
          Run <code className="rounded bg-muted px-1.5 py-0.5 text-xs">suggest-schema</code> to generate a schema proposal from your documents.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Header with status and actions */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <span className="text-sm font-medium">Status:</span>
          <Badge variant={schema.status === 'approved' ? 'default' : schema.status === 'rejected' ? 'destructive' : 'secondary'}>
            {schema.status}
          </Badge>
          {schema.sample_size != null && (
            <span className="text-xs text-muted-foreground">
              Proposed from {schema.sample_size} of {schema.total_documents} documents
            </span>
          )}
        </div>

        <div className="flex gap-2">
          {(isApproved && !editingApproved) || isProposed ? (
            <>
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  setShowPreviewModal(true);
                  resetPreview();
                  void fetchCostEstimate();
                }}
                disabled={isPreviewRunning || dirty}
                title={dirty ? 'Save changes before previewing' : undefined}
              >
                <Eye className="mr-1 size-3.5" /> Preview Extraction
              </Button>
              {isApproved && !editingApproved && (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setEditingApproved(true)}
                >
                  <Pencil className="mr-1 size-3.5" /> Edit Schema
                </Button>
              )}
            </>
          ) : null}
          {editingApproved && (
            <Button size="sm" variant="ghost" onClick={handleCancelEdit}>
              Cancel
            </Button>
          )}
          {isEditable && dirty && (
            <Button
              size="sm"
              variant="outline"
              onClick={handleSave}
              disabled={updateMutation.isPending}
            >
              {updateMutation.isPending ? 'Saving...' : 'Save & Re-propose'}
            </Button>
          )}
          {isProposed && (
            <>
              <Button
                size="sm"
                variant="destructive"
                onClick={() => rejectMutation.mutate()}
                disabled={rejectMutation.isPending}
              >
                {rejectMutation.isPending ? 'Rejecting...' : 'Reject'}
              </Button>
              <Button
                size="sm"
                onClick={handleApprove}
                disabled={dirty || approveMutation.isPending}
              >
                {approveMutation.isPending ? 'Approving...' : 'Approve Schema'}
              </Button>
            </>
          )}
        </div>
      </div>

      {(approveMutation.isError || rejectMutation.isError || updateMutation.isError) && (
        <div className="rounded-lg border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive">
          {approveMutation.error?.message ?? rejectMutation.error?.message ?? updateMutation.error?.message}
        </div>
      )}

      {isProposed && finalPreviewResult && !dirty && (
        <div className="rounded-lg border border-blue-200 bg-blue-50 p-3 text-sm text-blue-800 dark:border-blue-800 dark:bg-blue-950 dark:text-blue-200">
          Schema edits saved. Status reverted to <strong>proposed</strong>. Preview with your updated schema, then approve when satisfied.
        </div>
      )}

      {/* Summary cards */}
      <div className="grid grid-cols-2 gap-4">
        <div className="rounded-md border p-4">
          <div className="text-sm font-medium">Entity Types</div>
          <div className="mt-1 text-2xl font-bold">{currentEntities.length}</div>
        </div>
        <div className="rounded-md border p-4">
          <div className="text-sm font-medium">Relation Types</div>
          <div className="mt-1 text-2xl font-bold">{currentRelations.length}</div>
        </div>
      </div>

      <Separator />

      {/* Entity Types Table */}
      <div>
        <div className="mb-3 flex items-center justify-between">
          <h3 className="text-sm font-semibold">Entity Types</h3>
          {isEditable && (
            <Button variant="outline" size="sm" onClick={handleAddEntity} className="h-7 gap-1 text-xs">
              <Plus className="size-3" /> Add Entity
            </Button>
          )}
        </div>
        <div className="rounded-md border">
          <table className="w-full">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="p-2 text-left text-xs font-medium text-muted-foreground">Name</th>
                <th className="p-2 text-left text-xs font-medium text-muted-foreground">Description</th>
                <th className="p-2 text-center text-xs font-medium text-muted-foreground">Type</th>
                {isEditable && <th className="w-20 p-2" />}
              </tr>
            </thead>
            <tbody>
              {currentEntities.map((entity, i) => (
                <EntityRow
                  key={`${entity.name}-${i}`}
                  entity={entity}
                  isEditable={isEditable}
                  onRemove={() => updateEntities(currentEntities.filter((_, j) => j !== i))}
                  onUpdate={(updated) => updateEntities(currentEntities.map((e, j) => j === i ? updated : e))}
                  previewCount={finalPreviewResult ? entityCountMap.get(entity.name) ?? 0 : undefined}
                />
              ))}
              {currentEntities.length === 0 && (
                <tr>
                  <td colSpan={4} className="p-4 text-center text-sm text-muted-foreground">
                    No entity types defined
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>

      <Separator />

      {/* Relation Types Table */}
      <div>
        <div className="mb-3 flex items-center justify-between">
          <h3 className="text-sm font-semibold">Relation Types</h3>
          {isEditable && (
            <Button variant="outline" size="sm" onClick={handleAddRelation} className="h-7 gap-1 text-xs">
              <Plus className="size-3" /> Add Relation
            </Button>
          )}
        </div>
        <div className="rounded-md border">
          <table className="w-full">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="p-2 text-left text-xs font-medium text-muted-foreground">Name</th>
                <th className="p-2 text-left text-xs font-medium text-muted-foreground">Description</th>
                <th className="p-2 text-left text-xs font-medium text-muted-foreground">Direction</th>
                {isEditable && <th className="w-20 p-2" />}
              </tr>
            </thead>
            <tbody>
              {currentRelations.map((relation, i) => (
                <RelationRow
                  key={`${relation.name}-${i}`}
                  relation={relation}
                  isEditable={isEditable}
                  onRemove={() => updateRelations(currentRelations.filter((_, j) => j !== i))}
                  onUpdate={(updated) => updateRelations(currentRelations.map((r, j) => j === i ? updated : r))}
                  previewCount={finalPreviewResult ? relationCountMap.get(relation.name) ?? 0 : undefined}
                />
              ))}
              {currentRelations.length === 0 && (
                <tr>
                  <td colSpan={4} className="p-4 text-center text-sm text-muted-foreground">
                    No relation types defined
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>

      {/* Preview Extraction Modal */}
      <PreviewConfirmModal
        open={showPreviewModal}
        onOpenChange={setShowPreviewModal}
        costEstimate={costEstimate}
        isLoading={isCostLoading}
        onConfirm={() => void triggerPreview()}
        isPreviewRunning={isPreviewRunning}
        isTriggerPending={isTriggerPending}
        documentsCompleted={documentsCompleted}
        documentsTotal={documentsTotal}
        previewStatus={previewResult?.status}
        cliCommand={triggerStatus?.cliCommand}
        onCancel={() => {
          cancelPolling();
          setShowPreviewModal(false);
        }}
        costError={costError}
        triggerError={triggerError}
      />

      {/* Preview Results Slide-over Panel */}
      {finalPreviewResult && (
        <PreviewResultsPanel
          open={showResultsPanel}
          onOpenChange={(open) => {
            setShowResultsPanel(open);
            if (!open) {
              setFinalPreviewResult(null);
              setSchemaSavedAfterPreview(false);
            }
          }}
          result={finalPreviewResult}
          onRunAgain={() => {
            setShowResultsPanel(false);
            setFinalPreviewResult(null);
            setSchemaSavedAfterPreview(false);
            resetPreview();
            setShowPreviewModal(true);
            void fetchCostEstimate();
          }}
          isRunning={isPreviewRunning}
          isStale={schemaSavedAfterPreview}
          isDirty={dirty}
        />
      )}
    </div>
  );
}
