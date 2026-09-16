import type { CleanResult } from '../models/types';

export type CleanOutcome = 'success' | 'partial' | 'failed';

/**
 * Derives the aggregate cleanup state from the native result so every surface
 * presents a total failure or partial cleanup honestly.
 *
 * The three answers mean:
 * - `success`: the run removed every target the plan authorized.
 * - `partial`: the run removed less than that — a target failed, a target was
 *   only partly removed, or any target was skipped.
 * - `failed`: no target was reached at all, or every target failed.
 *
 * A skipped target is neither a failure nor a success: it was not removed and
 * nothing about it went wrong, so it never counts as a failure, and it never
 * lets a run with an untouched target claim a clean success.
 *
 * The backend's own counts win when the payload carries them: it is the side
 * that knows which targets did not fully clean, and it counts a partial target
 * even when its message is missing. The item inspection below stays as the
 * fallback for a payload recorded before those counts existed.
 */
export function cleanOutcome(
  result: Pick<CleanResult, 'items'> &
    Partial<Pick<CleanResult, 'partial_count' | 'failed_count' | 'skipped_count'>>
): CleanOutcome {
  if (result.items.length === 0) return 'failed';

  const skippedCount =
    result.skipped_count ?? result.items.filter((item) => item.status === 'skipped').length;
  const failedCount =
    result.failed_count ??
    result.items.filter((item) => item.status !== 'skipped' && (!item.success || item.status === 'failed'))
      .length;
  const partialCount =
    result.partial_count ??
    result.items.filter(
      (item) =>
        item.status !== 'skipped' &&
        (item.status === 'partial' ||
          (item.success && item.error_message != null && item.error_message.length > 0))
    ).length;

  if (failedCount === result.items.length) return 'failed';
  if (failedCount > 0 || partialCount > 0 || skippedCount > 0) return 'partial';
  return 'success';
}
