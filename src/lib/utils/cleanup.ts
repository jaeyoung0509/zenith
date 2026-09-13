import type { RiskTier, ScanItem } from '../models/types';

export type CleanupSortMode = 'size' | 'name' | 'modified';

/** The raw size figure a row reports, whether or not the item can be cleaned. */
export function reclaimableBytes(item: ScanItem): number {
  return item.size.allocated ?? item.size.logical;
}

/** Observed bytes on disk for this item; zero if the item does not exist. */
export function observedBytes(item: ScanItem): number {
  if (!item.exists) return 0;
  return reclaimableBytes(item);
}

/**
 * Backend-owned authority on whether an item allows cleanup.
 *
 * When `item.disposition` is present, it is the sole authority.
 * An item is cleanable only when eligibility is `auto_cleanable` or `reviewable`
 * and it has non-zero cleanable bytes.
 */
export function isCleanable(item: ScanItem): boolean {
  const eligibility = item.disposition?.eligibility;
  return (
    (eligibility === 'auto_cleanable' || eligibility === 'reviewable') &&
    cleanableBytes(item) > 0
  );
}

/** Whether the item is automatically cleanable by Safe/Quick Clean actions. */
export function isAutoCleanable(item: ScanItem): boolean {
  return item.disposition?.eligibility === 'auto_cleanable' && cleanableBytes(item) > 0;
}

/** Whether the item is explicitly blocked from generic cleanup (e.g. nested .app, inaccessible). */
export function isBlocked(item: ScanItem): boolean {
  return !item.disposition || item.disposition.eligibility === 'blocked';
}

/** Whether the item is advisory-only (external/manual management required). */
export function isAdvisory(item: ScanItem): boolean {
  return item.disposition?.eligibility === 'advisory';
}

/** Bytes this item would actually reclaim; zero when it cannot be cleaned. */
export function cleanableBytes(item: ScanItem): number {
  const eligibility = item.disposition?.eligibility;
  if (eligibility !== 'auto_cleanable' && eligibility !== 'reviewable') return 0;
  return Math.min(item.disposition?.cleanable_bytes ?? 0, observedBytes(item));
}

export interface CategorySummary {
  detected_count: number;
  visible_count: number;
  cleanable_count: number;
  selected_count: number;
  blocked_count: number;
  advisory_count: number;
  observed_bytes: number;
  cleanable_bytes: number;
  selected_bytes: number;
  is_all_cleanable_selected: boolean;
  can_select_all: boolean;
}

/**
 * Derives comprehensive, authoritative counts and byte totals for a category.
 *
 * Guaranteed invariant:
 * `selected_bytes <= cleanable_bytes <= observed_bytes`
 */
export function summarizeCategory(
  items: ScanItem[],
  selectedMap: Record<string, boolean> = {}
): CategorySummary {
  const detected_count = items.length;
  let visible_count = 0;
  let cleanable_count = 0;
  let selected_count = 0;
  let blocked_count = 0;
  let advisory_count = 0;
  let observed_bytes = 0;
  let cleanable_bytes_sum = 0;
  let selected_bytes = 0;

  for (const item of items) {
    const obs = observedBytes(item);
    observed_bytes += obs;

    if (isPresentedItem(item)) {
      visible_count++;
    }

    if (isBlocked(item)) {
      blocked_count++;
    } else if (isAdvisory(item)) {
      advisory_count++;
    }

    if (isCleanable(item)) {
      cleanable_count++;
      const cln = cleanableBytes(item);
      cleanable_bytes_sum += cln;
      if (selectedMap[item.id]) {
        selected_count++;
        selected_bytes += cln;
      }
    }
  }

  const is_all_cleanable_selected = cleanable_count > 0 && selected_count === cleanable_count;
  const can_select_all = cleanable_count > 0 && selected_count < cleanable_count;

  return {
    detected_count,
    visible_count,
    cleanable_count,
    selected_count,
    blocked_count,
    advisory_count,
    observed_bytes,
    cleanable_bytes: cleanable_bytes_sum,
    selected_bytes,
    is_all_cleanable_selected,
    can_select_all,
  };
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
