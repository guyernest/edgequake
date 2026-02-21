import { useState } from 'react';
import { Copy, Check } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { toast } from 'sonner';

interface CliCommandDisplayProps {
  command: string;
}

export function CliCommandDisplay({ command }: CliCommandDisplayProps) {
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      toast.success('Command copied to clipboard');
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error('Failed to copy command');
    }
  }

  return (
    <div className="relative rounded-md bg-gray-900 p-4">
      <pre className="overflow-x-auto whitespace-pre-wrap break-all font-mono text-sm text-green-400">
        {command}
      </pre>
      <Button
        variant="ghost"
        size="icon-sm"
        className="absolute top-2 right-2 text-gray-400 hover:text-white hover:bg-gray-800"
        onClick={handleCopy}
      >
        {copied ? (
          <Check className="size-4" />
        ) : (
          <Copy className="size-4" />
        )}
      </Button>
    </div>
  );
}
