import { useMutation, useQueryClient } from '@tanstack/react-query';
import { client } from '@/amplify-client';

export function useApproveSchema(slug: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const { data, errors } = await client.mutations.approveSchema({ slug });
      if (errors) throw new Error(errors.map((e) => e.message).join(', '));
      return data;
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['schemaProposal', slug] });
      void queryClient.invalidateQueries({ queryKey: ['pipelineConfig', slug] });
    },
  });
}

export function useRejectSchema(slug: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const { data, errors } = await client.mutations.rejectSchema({ slug });
      if (errors) throw new Error(errors.map((e) => e.message).join(', '));
      return data;
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['schemaProposal', slug] });
    },
  });
}

export function useUpdateSchemaTypes(slug: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (args: {
      entity_types?: unknown[];
      relation_types?: unknown[];
    }) => {
      const { data, errors } = await client.mutations.updateSchemaTypes({
        slug,
        entity_types: args.entity_types ? JSON.stringify(args.entity_types) : undefined,
        relation_types: args.relation_types ? JSON.stringify(args.relation_types) : undefined,
      });
      if (errors) throw new Error(errors.map((e) => e.message).join(', '));
      return data;
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['schemaProposal', slug] });
    },
  });
}
