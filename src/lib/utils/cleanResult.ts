import type { CleanResult } from '../models/types';

export type CleanOutcome = 'success' | 'partial' | 'failed';

/**
 * Derives the aggregate cleanup state from the native result so every surface
 * presents a total failure or partial cleanup honestly.
 */
export function cleanOutcome(result: Pick<CleanResult, 'items'>): CleanOutcome {
  if (result.items.length === 0) return 'failed';

  const failedItems = result.items.filter(
    (item) => !item.success || item.status === 'failed'
  );
  const partialItems = result.items.filter(
    (item) =>
      item.status === 'partial' ||
      (item.success && item.error_message != null && item.error_message.length > 0)
  );

  if (failedItems.length === result.items.length) return 'failed';
  if (failedItems.length > 0 || partialItems.length > 0) return 'partial';
  return 'success';
}
