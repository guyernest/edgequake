import { useState } from 'react';
import { toast } from 'sonner';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useCreateNamespace } from '@/hooks/useNamespaces';

const SLUG_PATTERN = /^[a-z0-9][a-z0-9-]*[a-z0-9]$/;
const MIN_LENGTH = 3;
const MAX_LENGTH = 50;

function validateSlug(value: string): string | null {
  if (value.length < MIN_LENGTH) {
    return `Name must be at least ${MIN_LENGTH} characters`;
  }
  if (value.length > MAX_LENGTH) {
    return `Name must be at most ${MAX_LENGTH} characters`;
  }
  if (!SLUG_PATTERN.test(value)) {
    return 'Name must contain only lowercase letters, numbers, and hyphens';
  }
  if (value.includes('--')) {
    return 'Name must not contain consecutive hyphens';
  }
  return null;
}

type CreateNamespaceDialogProps = {
  trigger: React.ReactNode;
};

export function CreateNamespaceDialog({ trigger }: CreateNamespaceDialogProps) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('');
  const [domainHint, setDomainHint] = useState('');
  const [validationError, setValidationError] = useState<string | null>(null);

  const createNamespace = useCreateNamespace();

  function resetForm() {
    setName('');
    setDomainHint('');
    setValidationError(null);
  }

  function handleNameChange(value: string) {
    const normalized = value.toLowerCase().replace(/[^a-z0-9-]/g, '');
    setName(normalized);
    if (normalized.length > 0) {
      setValidationError(validateSlug(normalized));
    } else {
      setValidationError(null);
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();

    const error = validateSlug(name);
    if (error) {
      setValidationError(error);
      return;
    }

    try {
      await createNamespace.mutateAsync({
        slug: name,
        description: domainHint || undefined,
      });
      toast.success(`Namespace "${name}" created`);
      resetForm();
      setOpen(false);
    } catch (err) {
      const message =
        err instanceof Error ? err.message : 'Failed to create namespace';
      toast.error(message);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) resetForm();
      }}
    >
      <DialogTrigger asChild>{trigger}</DialogTrigger>
      <DialogContent>
        <form
          onSubmit={(e) => {
            void handleSubmit(e);
          }}
        >
          <DialogHeader>
            <DialogTitle>Create Namespace</DialogTitle>
            <DialogDescription>
              A namespace isolates a complete Graph RAG pipeline including
              documents, knowledge graph, and vector embeddings.
            </DialogDescription>
          </DialogHeader>

          <div className="mt-4 flex flex-col gap-4">
            <div>
              <label
                htmlFor="namespace-name"
                className="mb-1.5 block text-sm font-medium"
              >
                Namespace Name
              </label>
              <Input
                id="namespace-name"
                placeholder="my-dataset"
                value={name}
                onChange={(e) => handleNameChange(e.target.value)}
                aria-invalid={validationError ? true : undefined}
                autoFocus
              />
              {validationError && (
                <p className="mt-1 text-sm text-destructive">
                  {validationError}
                </p>
              )}
              <p className="mt-1 text-xs text-muted-foreground">
                Lowercase letters, numbers, and hyphens. 3-50 characters.
              </p>
            </div>

            <div>
              <label
                htmlFor="domain-hint"
                className="mb-1.5 block text-sm font-medium"
              >
                Domain Hint
                <span className="ml-1 text-muted-foreground font-normal">
                  (optional)
                </span>
              </label>
              <textarea
                id="domain-hint"
                className="border-input bg-transparent placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px] min-h-[80px] w-full rounded-md border px-3 py-2 text-sm shadow-xs outline-none"
                placeholder="Describe the domain of your dataset (e.g., legal documents, financial reports, medical records)"
                value={domainHint}
                onChange={(e) => setDomainHint(e.target.value)}
              />
            </div>
          </div>

          <DialogFooter className="mt-6">
            <Button
              type="button"
              variant="outline"
              onClick={() => setOpen(false)}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={
                createNamespace.isPending ||
                name.length === 0 ||
                validationError !== null
              }
            >
              {createNamespace.isPending ? 'Creating...' : 'Create Namespace'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
