import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { Play } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { CliCommandDisplay } from './CliCommandDisplay';
import { client } from '@/amplify-client';

const ACTIVE_STATUSES = new Set([
  'running',
  'preparing',
  'extracting',
  'embedding',
  'storing',
  'requested',
]);

interface TriggerIngestionDialogProps {
  slug: string;
  pipelineStatus?: string | null;
  llmModel?: string;
  embeddingModel?: string;
}

export function TriggerIngestionDialog({
  slug,
  pipelineStatus,
  llmModel,
  embeddingModel,
}: TriggerIngestionDialogProps) {
  const [open, setOpen] = useState(false);
  const [dataPath, setDataPath] = useState('');
  const [batchSize, setBatchSize] = useState(1000);
  const [offset, setOffset] = useState(0);
  const queryClient = useQueryClient();

  const isActive = pipelineStatus
    ? ACTIVE_STATUSES.has(pipelineStatus)
    : false;

  const triggerMutation = useMutation({
    mutationFn: async () => {
      const { data, errors } = await client.mutations.triggerIngestion({
        slug,
        batch_size: batchSize,
        offset,
        data_path: dataPath,
      });
      if (errors) {
        throw new Error(errors.map((e) => e.message).join(', '));
      }
      return data;
    },
    onSuccess: () => {
      setOpen(false);
      toast.success('Ingestion triggered successfully');
      // Invalidate status to trigger polling
      queryClient.invalidateQueries({ queryKey: ['namespaceStatus', slug] });
    },
    onError: (error: Error) => {
      toast.error(`Failed to trigger ingestion: ${error.message}`);
    },
  });

  const cliCommand = [
    'edgequake-batch',
    `--namespace ${slug}`,
    `--data ${dataPath || '<data-path>'}`,
    `--limit ${batchSize}`,
    `--offset ${offset}`,
    ...(llmModel ? [`--model ${llmModel}`] : []),
    ...(embeddingModel ? [`--embedding-model ${embeddingModel}`] : []),
    'run',
  ].join(' \\\n  ');

  const isFormValid = dataPath.trim().length > 0 && batchSize > 0;

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button
          size="sm"
          disabled={isActive}
          title={
            isActive ? 'Pipeline running -- wait for completion' : undefined
          }
        >
          <Play className="size-4" />
          Start Ingestion
        </Button>
      </DialogTrigger>

      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Start Ingestion</DialogTitle>
          <DialogDescription>
            Trigger a new pipeline ingestion run for{' '}
            <span className="font-semibold text-foreground">{slug}</span>.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="data-path">Data Path</Label>
            <Input
              id="data-path"
              placeholder="/path/to/data.parquet or s3://bucket/key"
              value={dataPath}
              onChange={(e) => setDataPath(e.target.value)}
            />
            <p className="text-xs text-muted-foreground">
              Path to the parquet data file or S3 URI
            </p>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <Label htmlFor="batch-size">Batch Size</Label>
              <Input
                id="batch-size"
                type="number"
                min={1}
                max={100000}
                value={batchSize}
                onChange={(e) => setBatchSize(Number(e.target.value))}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="offset">Offset</Label>
              <Input
                id="offset"
                type="number"
                min={0}
                value={offset}
                onChange={(e) => setOffset(Number(e.target.value))}
              />
            </div>
          </div>

          <div className="rounded-md bg-muted/50 p-3 text-sm">
            Will process{' '}
            <span className="font-medium">{batchSize.toLocaleString()}</span>{' '}
            documents starting from offset{' '}
            <span className="font-medium">{offset.toLocaleString()}</span> in
            namespace <span className="font-medium">{slug}</span>
          </div>

          <div className="space-y-1.5">
            <Label>CLI Command</Label>
            <CliCommandDisplay command={cliCommand} />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button
            onClick={() => triggerMutation.mutate()}
            disabled={
              !isFormValid || isActive || triggerMutation.isPending
            }
          >
            {triggerMutation.isPending
              ? 'Starting...'
              : 'Start Ingestion'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
