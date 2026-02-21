import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { client } from '@/amplify-client';

export function usePipelineConfig(slug: string) {
  return useQuery({
    queryKey: ['pipelineConfig', slug],
    queryFn: async () => {
      const { data, errors } =
        await client.queries.getNamespacePipelineConfig({ slug });
      if (errors) {
        throw new Error(errors.map((e) => e.message).join(', '));
      }
      return data;
    },
    staleTime: 5 * 60 * 1000,
  });
}

/**
 * Mutation hook for updating pipeline configuration.
 * Invalidates the pipeline-config query on success and shows a toast.
 */
export function useUpdatePipelineConfig(slug: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (config: Record<string, unknown>) => {
      const { data, errors } = await client.mutations.updatePipelineConfig({
        slug,
        config: JSON.stringify(config),
      });
      if (errors) {
        throw new Error(errors.map((e) => e.message).join(', '));
      }
      return data;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['pipelineConfig', slug] });
      toast.success('Pipeline configuration saved');
    },
    onError: (error: Error) => {
      toast.error(`Failed to save configuration: ${error.message}`);
    },
  });
}
