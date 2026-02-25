import { useQuery } from '@tanstack/react-query';
import { edgequakeApi } from '@/lib/api-client';

export interface EntityItem {
  name: string;
  entity_type: string;
  degree: number;
  description: string | null;
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
    queryFn: () => {
      const params = new URLSearchParams();
      params.set('page', String(page));
      params.set('page_size', '25');
      if (search) params.set('search', search);
      if (entityType) params.set('entity_type', entityType);
      return edgequakeApi.get<EntityListResponse>(
        `/api/v1/ns/${slug}/graph/entities?${params.toString()}`,
      );
    },
    enabled: !!slug,
  });
}
