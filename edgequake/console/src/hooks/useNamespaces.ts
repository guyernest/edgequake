import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { client } from '@/amplify-client';

export function useNamespaces() {
  return useQuery({
    queryKey: ['namespaces'],
    queryFn: async () => {
      const { data, errors } = await client.queries.listNamespaces();
      if (errors) {
        throw new Error(errors.map((e) => e.message).join(', '));
      }
      return data ?? [];
    },
  });
}

export function useCreateNamespace() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      slug,
      description,
    }: {
      slug: string;
      description?: string;
    }) => {
      const { data, errors } = await client.mutations.createNamespace({
        slug,
        description,
      });
      if (errors) {
        throw new Error(errors.map((e) => e.message).join(', '));
      }
      return data;
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['namespaces'] });
    },
  });
}
