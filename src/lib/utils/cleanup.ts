import type { CategoryResult, RiskTier, ScanItem } from '../models/types';

export type CleanupSortMode = 'size' | 'name' | 'modified';

export interface ByteRange {
  lower: number;
  upper: number;
  isAmbiguous: boolean;
}

/**
 * The observed union when some nested observations may already be part of a
 * broader measurement. totalBytes is the conservative upper bound and the
 * ambiguous population is subtracted for the lower bound.
 */
export function observedByteRange(totalBytes: number, ambiguousOverlapBytes = 0): ByteRange {
  const upper = Math.max(0, totalBytes);
  const ambiguous = Math.min(upper, Math.max(0, ambiguousOverlapBytes));
  return {
    lower: upper - ambiguous,
    upper,
    isAmbiguous: ambiguous > 0,
  };
}

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
  if (eligibility !== 'auto_cleanable' && eligibility !== 'reviewable') return false;
  if (cleanableBytes(item) > 0) return true;

  // A package-store command may prune unused entries, but the measured store
  // footprint is not a reclaim estimate. Keep it explicitly selectable with
  // zero bytes in the guaranteed totals.
  return (
    eligibility === 'reviewable' &&
    item.exists &&
    observedBytes(item) > 0 &&
    item.cache_metadata?.management_mode === 'tool_managed' &&
    item.cache_metadata.artifact_kind === 'package_store'
  );
}

/** Whether the backend includes this cache in direct cleanup. */
export function isAutoCleanable(item: ScanItem): boolean {
  return item.disposition?.eligibility === 'auto_cleanable' && cleanableBytes(item) > 0;
}

/** Whether a reviewed lifecycle provider runs this item's cleanup instead of generic deletion. */
export function isProviderBacked(item: ScanItem): boolean {
  return item.lifecycle_provider_action === true;
}

/**
 * Whether the submission path may hand this item to cleanup.
 *
 * The manual tier is refused generic cleanup because those items name a
 * management action Zenith does not own. A provider-backed manual item is the
 * one exception: its signature names the operation that will run, so the
 * reviewed provider performs it like any other cleanable row.
 */
export function isActionable(item: ScanItem): boolean {
  return isCleanable(item) && (item.risk !== 'manual' || isProviderBacked(item));
}

/** Whether the item is explicitly blocked from generic cleanup (e.g. nested .app, inaccessible). */
export function isBlocked(item: ScanItem): boolean {
  return !item.disposition || item.disposition.eligibility === 'blocked';
}

/** Whether the item is advisory-only (external/manual management required). */
export function isAdvisory(item: ScanItem): boolean {
  return item.disposition?.eligibility === 'advisory';
}

/** Whether the item was discovered but the age policy does not authorize it yet. */
export function isRecent(item: ScanItem): boolean {
  return item.disposition?.eligibility === 'recent';
}

/** Whether the item was discovered but the current settings refuse to clean it. */
export function isPolicyGated(item: ScanItem): boolean {
  return item.disposition?.eligibility === 'policy_gated';
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
  recent_count: number;
  policy_gated_count: number;
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
  let recent_count = 0;
  let policy_gated_count = 0;
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
    } else if (isRecent(item)) {
      recent_count++;
    } else if (isPolicyGated(item)) {
      policy_gated_count++;
    }

    if (isActionable(item)) {
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
    recent_count,
    policy_gated_count,
    observed_bytes,
    cleanable_bytes: cleanable_bytes_sum,
    selected_bytes,
    is_all_cleanable_selected,
    can_select_all,
  };
}

/** Compact explanation for a category with no currently actionable item. */
export function emptyCategoryMessage(
  summary: CategorySummary,
  quality: CategoryResult['quality'],
  category?: CategoryResult['category']
): string | null {
  if (summary.cleanable_count > 0) return null;
  if (summary.visible_count === 0) {
    return quality === 'fresh' ? 'No items found' : 'Some locations unavailable';
  }
  if (category === 'container') return 'Manage containers';
  if (summary.advisory_count === summary.visible_count) return 'Managed elsewhere';
  if (summary.policy_gated_count > 0) return 'Review scan scope';
  if (summary.recent_count > 0) return 'Nothing ready yet';
  if (summary.blocked_count > 0) return 'Needs attention';
  return 'No items eligible';
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

export interface RowTotals {
  count: number;
  bytes: number;
}

/**
 * Detected versus currently cleanable totals for one row set (a tab's rows).
 *
 * A tab counts risk classification while the actions act on current
 * eligibility, so the two numbers can disagree: a tab can hold twelve safe rows
 * and authorize none of them. Both sides read the same predicates the actions
 * use, so a view can state the difference without re-deriving backend policy.
 */
export interface CleanableTotals {
  detected: RowTotals;
  cleanable: RowTotals;
}

export function cleanableTotals(items: ScanItem[]): CleanableTotals {
  const totals: CleanableTotals = {
    detected: { count: 0, bytes: 0 },
    cleanable: { count: 0, bytes: 0 },
  };
  for (const item of items) {
    totals.detected.count++;
    totals.detected.bytes += observedBytes(item);
    if (isCleanable(item)) {
      totals.cleanable.count++;
      totals.cleanable.bytes += cleanableBytes(item);
    }
  }
  return totals;
}

/** The eligibility states that keep a discovered row out of the cleanup actions. */
export type IneligibleState = 'blocked' | 'advisory' | 'recent' | 'outside_scope' | 'incomplete';

export interface IneligibleStateCount {
  state: IneligibleState;
  /** How the state reads inside a sentence, e.g. "recently used". */
  label: string;
  count: number;
}

const INELIGIBLE_LABELS: Record<IneligibleState, string> = {
  blocked: 'blocked',
  advisory: 'advisory',
  recent: 'recently used',
  outside_scope: 'outside the current scope',
  incomplete: 'not fully measured',
};

const INELIGIBLE_ORDER: IneligibleState[] = [
  'blocked',
  'advisory',
  'recent',
  'outside_scope',
  'incomplete',
];

/**
 * The one state that keeps this row out of cleanup, or `null` when it is
 * cleanable. One state per row, read in the order the disposition is read
 * everywhere else, so a row that is both blocked and unmeasured is counted once.
 */
function ineligibleState(item: ScanItem): IneligibleState | null {
  if (isCleanable(item)) return null;
  if (isBlocked(item)) return 'blocked';
  if (isAdvisory(item)) return 'advisory';
  if (isRecent(item)) return 'recent';
  if (isPolicyGated(item)) return 'outside_scope';
  if (item.quality !== 'fresh' || item.incomplete_reason) return 'incomplete';
  return null;
}

/**
 * The states keeping rows out of the cleanup actions, counted by state.
 *
 * A set with at least one cleanable row has something to select; this is what a
 * view states when it does not, and which rows a warning is about when it does.
 */
export function ineligibleStates(items: ScanItem[]): IneligibleStateCount[] {
  const counts: Partial<Record<IneligibleState, number>> = {};
  for (const item of items) {
    const state = ineligibleState(item);
    if (!state) continue;
    counts[state] = (counts[state] ?? 0) + 1;
  }
  return INELIGIBLE_ORDER.filter((state) => counts[state]).map((state) => ({
    state,
    label: INELIGIBLE_LABELS[state],
    count: counts[state] ?? 0,
  }));
}

/** How ineligible states read in one sentence, e.g. "3 recently used, 1 advisory". */
export function describeIneligibleStates(states: IneligibleStateCount[]): string {
  return states.map((entry) => `${entry.count} ${entry.label}`).join(', ');
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
