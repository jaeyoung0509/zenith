export interface VirtualWindow {
  start: number;
  end: number;
  offsetTop: number;
  offsetBottom: number;
}

/**
 * Returns the small contiguous slice that should be mounted for a fixed-height
 * scroll list. Overscan keeps keyboard and wheel scrolling from exposing blank
 * space while the next Svelte update is scheduled.
 */
export function getVirtualWindow(
  itemCount: number,
  rowHeight: number,
  scrollTop: number,
  viewportHeight: number,
  overscan = 4,
): VirtualWindow {
  const count = Math.max(0, Math.floor(itemCount));
  const height = Math.max(1, rowHeight);
  const top = Number.isFinite(scrollTop) ? Math.max(0, scrollTop) : 0;
  const viewport = Number.isFinite(viewportHeight) ? Math.max(0, viewportHeight) : 0;
  const buffer = Math.max(0, Math.floor(overscan));

  const firstVisible = Math.min(count, Math.floor(top / height));
  const lastVisible = Math.min(count, Math.ceil((top + viewport) / height));
  const start = Math.max(0, firstVisible - buffer);
  const end = Math.min(count, Math.max(start, lastVisible + buffer));

  return {
    start,
    end,
    offsetTop: start * height,
    offsetBottom: (count - end) * height,
  };
}
