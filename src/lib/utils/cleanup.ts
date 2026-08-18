import type { RiskTier, ScanItem } from '../models/types';

export type CleanupSortMode = 'size' | 'name' | 'modified';

export function reclaimableBytes(item: ScanItem): number {
  return item.size.allocated ?? item.size.logical;
}

export function filterAndSortCleanupItems(
  items: ScanItem[],
  risk: RiskTier | 'all',
  query: string,
  sort: CleanupSortMode
): ScanItem[] {
  const normalizedQuery = query.trim().toLowerCase();
  return items
    .filter((item) => {
      if (!item.exists || reclaimableBytes(item) <= 0) return false;
      if (risk !== 'all' && item.risk !== risk) return false;
      return (
        !normalizedQuery ||
        item.name.toLowerCase().includes(normalizedQuery) ||
        item.path.toLowerCase().includes(normalizedQuery) ||
        item.description.toLowerCase().includes(normalizedQuery)
      );
    })
    .sort((left, right) => {
      if (sort === 'name') return left.name.localeCompare(right.name);
      if (sort === 'modified') return (right.last_modified ?? 0) - (left.last_modified ?? 0);
      return reclaimableBytes(right) - reclaimableBytes(left) || left.name.localeCompare(right.name);
    });
}
