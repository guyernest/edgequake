import { useState, useEffect } from 'react';
import { useQuery, useMutation } from '@tanstack/react-query';
import { client } from '@/amplify-client';

/**
 * Hook for preview extraction flow: cost estimation, trigger, and result polling.
 *
 * Connects to the AppSync estimatePreviewCost query, triggerPreviewExtraction
 * mutation, and getPreviewResult polling query.
 */
export function usePreviewExtraction(namespace: string) {
  const [isPolling, setIsPolling] = useState(false);

  // Cost estimation query (only fetched when explicitly triggered)
  const costEstimate = useQuery({
    queryKey: ['preview-cost', namespace],
    queryFn: async () => {
      const { data, errors } = await client.queries.estimatePreviewCost({
        namespace,
      });
      if (errors) throw new Error(errors.map((e) => e.message).join(', '));
      return data;
    },
    enabled: false, // Only fetch when explicitly triggered via refetch
  });

  // Trigger preview mutation (writes PREVIEW_REQUEST to DynamoDB)
  const triggerMutation = useMutation({
    mutationFn: async () => {
      const { data, errors } =
        await client.mutations.triggerPreviewExtraction({ namespace });
      if (errors) throw new Error(errors.map((e) => e.message).join(', '));
      return data;
    },
    onSuccess: () => setIsPolling(true),
  });

  // Poll for preview results (every 2 seconds while isPolling is true)
  const previewResult = useQuery({
    queryKey: ['preview-result', namespace],
    queryFn: async () => {
      const { data, errors } = await client.queries.getPreviewResult({
        namespace,
      });
      if (errors) throw new Error(errors.map((e) => e.message).join(', '));
      return data;
    },
    enabled: isPolling,
    refetchInterval: isPolling ? 2000 : false,
    refetchIntervalInBackground: false,
  });

  // Stop polling when result status is terminal
  useEffect(() => {
    const status = previewResult.data?.status;
    if (
      status === 'completed' ||
      status === 'failed' ||
      status === 'cancelled'
    ) {
      setIsPolling(false);
    }
  }, [previewResult.data?.status]);

  return {
    // Cost estimation
    fetchCostEstimate: costEstimate.refetch,
    costEstimate: costEstimate.data,
    isCostLoading: costEstimate.isFetching,
    costError: costEstimate.error,

    // Trigger
    triggerPreview: triggerMutation.mutateAsync,
    triggerStatus: triggerMutation.data,
    isTriggerPending: triggerMutation.isPending,
    triggerError: triggerMutation.error,

    // Polling results
    previewResult: previewResult.data,
    isPreviewRunning: isPolling,
    documentsCompleted: previewResult.data?.documentsCompleted ?? 0,
    documentsTotal: previewResult.data?.documentsTotal ?? 0,

    // Cancel polling (captures partial results from last poll)
    cancelPolling: () => {
      setIsPolling(false);
    },

    // Reset for "Run Again"
    resetPreview: () => {
      triggerMutation.reset();
      setIsPolling(false);
    },
  };
}
