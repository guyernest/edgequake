import { useQuery } from '@tanstack/react-query';
import { edgequakeApi } from '@/lib/api-client';

export interface RawEntityItem {
  id: string;
  entity_name: string;
  entity_type: string;
  degree: number;
  description: string | null;
}

export interface EntityItem {
  name: string;
  entity_type: string;
  degree: number;
  description: string | null;
}

interface RawEntityListResponse {
  items: RawEntityItem[];
  total: number;
  page: number;
  page_size: number;
  total_pages: number;
}

export interface EntityListResponse {
  items: EntityItem[];
  total: number;
  page: number;
  page_size: number;
  total_pages: number;
}

/**
 * Fetches a paginated, filterable list of entities for a namespace.
 *
 * @param slug - Namespace slug
 * @param search - Free-text search term (matched against name/description)
 * @param entityType - Entity type filter (empty string = all types)
 * @param page - 1-based page number
 */
export function useEntityList(
  slug: string,
  search: string,
  entityType: string,
  page: number,
) {
  return useQuery({
    queryKey: ['ns-entities', slug, search, entityType, page],
    queryFn: async () => {
      const params = new URLSearchParams();
      params.set('page', String(page));
      params.set('page_size', '25');
      if (search) params.set('search', search);
      if (entityType) params.set('entity_type', entityType);
      const raw = await edgequakeApi.get<RawEntityListResponse>(
        `/api/v1/ns/${slug}/graph/entities?${params.toString()}`,
      );
      return {
        ...raw,
        items: raw.items.map((item) => ({
          name: item.entity_name || item.id,
          entity_type: item.entity_type,
          degree: item.degree,
          description: item.description,
        })),
      } as EntityListResponse;
    },
    enabled: !!slug,
  });
}
