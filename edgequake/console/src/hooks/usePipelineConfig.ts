import { useQuery } from '@tanstack/react-query';
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
