/** Owns one interval for a subscriber, pausing it while the document is hidden. */
export function observeWhileVisible(tick: () => void, intervalMs: number): () => void {
  if (typeof document === 'undefined') return () => {};

  const page = document;
  let timer: ReturnType<typeof setInterval> | null = null;
  const stopTimer = () => {
    if (timer !== null) clearInterval(timer);
    timer = null;
  };
  const syncVisibility = () => {
    if (page.visibilityState !== 'visible') {
      stopTimer();
      return;
    }
    if (timer !== null) return;
    tick();
    timer = setInterval(tick, intervalMs);
  };

  page.addEventListener('visibilitychange', syncVisibility);
  syncVisibility();

  return () => {
    page.removeEventListener('visibilitychange', syncVisibility);
    stopTimer();
  };
}
