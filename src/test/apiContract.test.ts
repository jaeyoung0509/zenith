import { describe, expect, it, vi } from 'vitest';
import { mockApi } from '../lib/api/mock';
import { nativeApi } from '../lib/api/native';
import { mockStorageApi, nativeStorageApi } from '../lib/api/storage';
import type { ScanEvent } from '../lib/models/types';

describe('browser preview API parity', () => {
  it('implements exactly the native top-level command surface', () => {
    expect(Object.keys(mockApi).sort()).toEqual(Object.keys(nativeApi).sort());
  });

  it('implements exactly the native storage workflow surface', () => {
    expect(Object.keys(mockStorageApi).sort()).toEqual(
      Object.keys(nativeStorageApi).sort()
    );
  });
});

describe('browser preview scan cancellation', () => {
  /** Runs `body` with fake timers, so a preview scan's steps can be advanced. */
  async function withFakeTimers(body: () => Promise<void>): Promise<void> {
    vi.useFakeTimers();
    try {
      await body();
    } finally {
      vi.useRealTimers();
    }
  }

  async function startedScanId(events: ScanEvent[]): Promise<string> {
    const started = events.find((event) => event.type === 'Started');
    if (started?.type !== 'Started') throw new Error('The preview scan reported no scan id');
    return started.scan_id;
  }

  it('stops the running preview scan where it stands and reports it as cancelled', async () => {
    await withFakeTimers(async () => {
      const events: ScanEvent[] = [];
      const scan = mockApi.startScan((event) => {
        events.push(event);
      });
      await vi.advanceTimersByTimeAsync(150);
      expect(events.some((event) => event.type === 'ItemFound')).toBe(true);

      // The progress the UI reads: the root being read is named before the
      // category reports what it holds.
      const root = events.findIndex((event) => event.type === 'RootStarted');
      const category = events.findIndex((event) => event.type === 'CategoryStarted');
      expect(root).toBeGreaterThanOrEqual(0);
      expect(root).toBeLessThan(category);

      await mockApi.cancelScan(await startedScanId(events));
      await vi.advanceTimersByTimeAsync(450);

      const result = await scan;
      expect(result.cancelled).toBe(true);
      expect(result.quality).toBe('partial');
      expect(result.incomplete_reasons?.some((reason) => reason.includes('cancelled'))).toBe(true);
      expect(result.gaps).toEqual([{ kind: 'cancelled', count: 1 }]);
      // Only the category the walk finished is reported; the ones it never
      // reached were neither streamed nor counted.
      expect(result.categories.map((category) => category.category)).toEqual(['ai']);
      expect(events.some(
        (event) => event.type === 'CategoryStarted' && event.category === 'developer'
      )).toBe(false);
      expect(events.some(
        (event) => event.type === 'Finished' && event.result.cancelled === true
      )).toBe(true);
      expect(result.total_bytes).toBe(result.categories[0].total_bytes);
    });
  });

  it('leaves a scan that finished before the request alone', async () => {
    await withFakeTimers(async () => {
      const events: ScanEvent[] = [];
      const scan = mockApi.startScan((event) => {
        events.push(event);
      });
      await vi.advanceTimersByTimeAsync(450);
      const result = await scan;

      await expect(mockApi.cancelScan(result.scan_id)).resolves.toBeUndefined();

      expect(result.cancelled).toBe(false);
      expect(result.incomplete_reasons?.some((reason) => reason.includes('cancelled'))).toBe(false);
      expect(result.categories.map((category) => category.category)).toEqual([
        'ai',
        'developer',
        'system',
      ]);
    });
  });
});
