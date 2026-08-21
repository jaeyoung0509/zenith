import type {
  AppUninstallInspection,
  LargeFileItem,
  LargeFileKind,
} from '../models/types';

export const LARGE_FILE_MIN_BYTES = 100 * 1024 * 1024;
export const LARGE_FILE_DEFAULT_THRESHOLD_BYTES = 500 * 1024 * 1024;

export const LARGE_FILE_ROOTS = [
  { id: 'downloads', label: 'Downloads' },
  { id: 'desktop', label: 'Desktop' },
  { id: 'documents', label: 'Documents' },
  { id: 'movies', label: 'Movies' },
] as const;

export function clampLargeFileThreshold(bytes: number): number {
  if (!Number.isFinite(bytes)) return LARGE_FILE_DEFAULT_THRESHOLD_BYTES;
  return Math.max(LARGE_FILE_MIN_BYTES, Math.floor(bytes));
}

export function selectedLargeFileBytes(items: LargeFileItem[], selectedIds: string[]): number {
  const selected = new Set(selectedIds);
  return items.reduce(
    (sum, item) => sum + (selected.has(item.id) ? item.allocated_size : 0),
    0
  );
}

export function defaultRelatedIds(inspection: AppUninstallInspection): string[] {
  return inspection.related_items
    .filter((item) => item.confidence === 'high' && item.selected_by_default)
    .map((item) => item.id);
}

export function selectedAppTrashBytes(
  inspection: AppUninstallInspection,
  selectedRelatedIds: string[]
): number {
  const selected = new Set(selectedRelatedIds);
  return (
    inspection.app.allocated_size +
    inspection.related_items.reduce(
      (sum, item) => sum + (selected.has(item.id) ? item.allocated_size : 0),
      0
    )
  );
}

export function largeFileKindLabel(kind: LargeFileKind): string {
  switch (kind) {
    case 'video':
      return 'Video';
    case 'archive':
      return 'Archive';
    case 'disk_image':
      return 'Disk Image';
    case 'vm_image':
      return 'VM Image';
    case 'ai_model':
      return 'AI Model';
    case 'database':
      return 'Database';
    case 'developer_artifact':
      return 'Developer Artifact';
    default:
      return 'Other';
  }
}
