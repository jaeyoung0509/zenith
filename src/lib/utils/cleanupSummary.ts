export type CleanupSummaryState =
  | 'unavailable'
  | 'unknown'
  | 'scanning'
  | 'refreshing'
  | 'cleaning'
  | 'failed'
  | 'stale'
  | 'partial'
  | 'ready'
  | 'clean';

interface CleanupSummaryFacts {
  available: boolean;
  hasScan: boolean;
  scanning: boolean;
  cleaning: boolean;
  freshness: 'empty' | 'fresh' | 'partial' | 'unavailable' | 'stale' | 'refreshing' | 'failed';
  cleanableBytes: number;
}

/** Shared presentation phase for the Overview and Quick Panel summaries. */
export function cleanupSummaryState(facts: CleanupSummaryFacts): CleanupSummaryState {
  if (!facts.available) return 'unavailable';
  if (facts.cleaning) return 'cleaning';
  if (facts.scanning) return facts.hasScan ? 'refreshing' : 'scanning';
  if (facts.freshness === 'failed') return 'failed';
  if (!facts.hasScan) return 'unknown';
  if (facts.freshness === 'stale' || facts.freshness === 'unavailable') return 'stale';
  if (facts.freshness === 'partial') return 'partial';
  return facts.cleanableBytes > 0 ? 'ready' : 'clean';
}
