import type { RiskTier, ScanItem } from '../models/types';

export type CleanupSortMode = 'size' | 'name' | 'modified';

/** The raw size figure a row reports, whether or not the item can be cleaned. */
export function reclaimableBytes(item: ScanItem): number {
  return item.size.allocated ?? item.size.logical;
}

/**
 * Mirrors the backend's `ScanItem::is_cleanable_candidate`: an item is
 * cleanable only when its observation supports a deletion, its declared risk is
 * not `manual`, and it measured something to reclaim.
 */
export function isCleanable(item: ScanItem): boolean {
  return (item.quality === 'fresh' || item.quality === 'partial')
    && item.risk !== 'manual'
    && reclaimableBytes(item) > 0;
}

/** Bytes this item would actually reclaim; zero when it cannot be cleaned. */
export function cleanableBytes(item: ScanItem): number {
  return isCleanable(item) ? reclaimableBytes(item) : 0;
}

/**
 * Whether a cleanup view shows a row for this item.
 *
 * A cleanup view lists what it can measure plus what it must explain, and never
 * a complete observation of an empty path. This is the single predicate for
 * "is this a row", so headers, filter tabs, and rendered rows share one set.
 */
export function isPresentedItem(item: ScanItem): boolean {
  return item.exists && (reclaimableBytes(item) > 0 || item.quality !== 'fresh');
}

/** The row set every count, tab, and filter reads. */
export function presentedItems(items: ScanItem[]): ScanItem[] {
  return items.filter(isPresentedItem);
}

/** Counts per risk tier over the presented rows, so header and tabs agree. */
export function riskCounts(
  items: ScanItem[]
): { all: number; safe: number; rebuild: number; manual: number } {
  const counts = { all: 0, safe: 0, rebuild: 0, manual: 0 };
  for (const item of presentedItems(items)) {
    counts.all++;
    counts[item.risk]++;
  }
  return counts;
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
      if (!isPresentedItem(item)) return false;
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
