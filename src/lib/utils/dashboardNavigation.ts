/** Normalize persisted dashboard IDs while keeping old settings compatible. */
export function normalizeDashboardTab(tab: string): string {
  return tab === 'disk' ? 'disks' : tab;
}
