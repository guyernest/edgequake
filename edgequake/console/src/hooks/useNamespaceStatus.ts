import { useQuery } from '@tanstack/react-query';
import { client } from '@/amplify-client';

const ACTIVE_STATUSES = new Set([
  'running',
  'preparing',
  'extracting',
  'embedding',
  'storing',
]);

const ACTIVE_POLL_INTERVAL = 5_000;
const IDLE_POLL_INTERVAL = 30_000;

export function useNamespaceStatus(slug: string) {
  return useQuery({
    queryKey: ['namespaceStatus', slug],
    queryFn: async () => {
      const { data, errors } = await client.queries.getNamespaceStatus({
        slug,
      });
      if (errors) {
        throw new Error(errors.map((e) => e.message).join(', '));
      }
      return data;
    },
    refetchInterval: (query) => {
      const status = query.state.data?.status;
      if (status && ACTIVE_STATUSES.has(status)) {
        return ACTIVE_POLL_INTERVAL;
      }
      return IDLE_POLL_INTERVAL;
    },
    refetchIntervalInBackground: false,
  });
}
