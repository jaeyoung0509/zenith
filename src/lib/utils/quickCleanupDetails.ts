import type { ScanGapKind, ScanResult } from '../models/types';
import { isActionable, isAutoCleanable } from './cleanup';

const gapLabels: Record<ScanGapKind, string> = {
  full_disk_access: 'macOS protected locations; review Full Disk Access in Storage',
  permission_denied: 'Locations could not be read with the current permissions',
  depth_limit: 'Locations reached the scan depth limit',
  cancelled: 'Locations were not finished before the scan stopped',
  io_error: 'Locations could not be read',
};

/** Presentation of backend facts only; this does not authorize cleanup. */
export function quickCleanupDetails(scan: ScanResult, quickEligibleCount: number) {
  const items = scan.categories.flatMap(category => category.items);
  const reviewCount = items.filter(item => isActionable(item) && !isAutoCleanable(item)).length;
  const excludedAutomaticCount = Math.max(0, items.filter(isAutoCleanable).length - quickEligibleCount);
  const count = (eligibility: string) => items.filter(item => item.disposition?.eligibility === eligibility).length;
  return {
    reviewCount,
    excludedAutomaticCount,
    blockedCount: count('blocked'),
    recentCount: count('recent'),
    advisoryCount: count('advisory'),
    policyGatedCount: count('policy_gated'),
    gaps: (scan.gaps ?? []).filter(gap => gap.count > 0).map(gap => ({
      kind: gap.kind, count: gap.count, label: gapLabels[gap.kind],
    })),
  };
}
