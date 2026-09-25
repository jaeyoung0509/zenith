import type { DashboardTab } from '../models/types';

/**
 * Sidebar order for a fresh install: the control tower first, then the three
 * work areas, then the Tools group, with Settings pinned at the bottom of the
 * shell rather than in this list.
 */
export const DEFAULT_DASHBOARD_TABS: DashboardTab[] = [
  'overview',
  'storage',
  'performance',
  'projects',
  'docker',
  'models',
  'development_servers',
  'awake',
];

/**
 * Normalizes a persisted dashboard tab id. `disk` predates the Disks sub-tab
 * and `memory` predates the Performance page, so both keep working for saved
 * settings and deep links.
 */
export function normalizeDashboardTab(tab: string): string {
  if (tab === 'disk') return 'disks';
  if (tab === 'memory') return 'performance';
  return tab;
}
