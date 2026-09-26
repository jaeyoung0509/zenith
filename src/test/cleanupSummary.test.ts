import { describe, expect, it } from 'vitest';
import { cleanupSummaryState } from '../lib/utils/cleanupSummary';

const base = {
  available: true,
  hasScan: true,
  scanning: false,
  cleaning: false,
  freshness: 'fresh' as const,
  cleanableBytes: 0,
};

describe('shared cleanup summary phase', () => {
  it('reserves zero for a fresh completed inventory', () => {
    expect(cleanupSummaryState(base)).toBe('clean');
    expect(cleanupSummaryState({ ...base, freshness: 'stale' })).toBe('stale');
    expect(cleanupSummaryState({ ...base, freshness: 'failed' })).toBe('failed');
    expect(cleanupSummaryState({ ...base, freshness: 'partial' })).toBe('partial');
    expect(cleanupSummaryState({ ...base, hasScan: false, freshness: 'empty' })).toBe('unknown');
  });

  it('keeps cleaning, scanning, and post-clean measurement distinct', () => {
    expect(cleanupSummaryState({ ...base, cleaning: true })).toBe('cleaning');
    expect(cleanupSummaryState({ ...base, scanning: true })).toBe('refreshing');
    expect(cleanupSummaryState({ ...base, scanning: true, hasScan: false })).toBe('scanning');
    expect(cleanupSummaryState({ ...base, cleanableBytes: 4096 })).toBe('ready');
    expect(cleanupSummaryState({ ...base, available: false })).toBe('unavailable');
  });
});
