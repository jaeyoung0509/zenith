import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ScanStore } from '../lib/stores/scan.svelte';
import type { ScanResult } from '../lib/models/types';
import { tauriCreatePlan, tauriExecuteClean, tauriGetLastScan, tauriScan } from '../lib/utils/tauri';

vi.mock('../lib/utils/tauri', () => ({
  tauriCreatePlan: vi.fn(), tauriExecuteClean: vi.fn(),
  tauriGetLastScan: vi.fn(), tauriScan: vi.fn(),
}));

function fixture(id = 'scan', finished = 1000): ScanResult {
  return {
    scan_id: id, valid_for_seconds: 300, started_at: finished - 1, finished_at: finished,
    total_bytes: 10, safe_bytes: 10, rebuild_bytes: 0, manual_bytes: 0,
    categories: [{ category: 'developer', display_name: 'Developer', total_bytes: 10,
      safe_bytes: 10, rebuild_bytes: 0, manual_bytes: 0, items: [{
        id: `${id}-item`, signature_id: 'fixture', name: 'Fixture', category: 'developer',
        risk: 'safe', path: '/fixture', size: { logical: 10, allocated: 10 },
        file_count: 1, description: 'Test only', is_selected: false, last_modified: null, exists: true,
      }] }],
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(1000_000);
  vi.resetAllMocks();
});
afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

async function loaded() {
  const store = new ScanStore();
  vi.mocked(tauriGetLastScan).mockResolvedValue(fixture());
  await store.init();
  return store;
}

describe('cleanup freshness and recovery', () => {
  it('expires exactly at the backend lifetime and clears executable selections', async () => {
    const store = await loaded();
    vi.setSystemTime(1299_000);
    store.updateFreshness();
    expect(store.canClean).toBe(true);
    vi.setSystemTime(1300_000);
    store.updateFreshness();
    expect(store.freshness).toBe('stale');
    expect(store.selectedCount).toBe(0);
    expect(store.lastScan?.total_bytes).toBe(10);
  });

  it('fails closed after a backward clock change', async () => {
    const store = await loaded();
    vi.setSystemTime(999_000);
    store.updateFreshness();
    expect(store.canClean).toBe(false);
  });

  it('blocks a stale click even without a preceding timer tick', async () => {
    const store = await loaded();
    vi.setSystemTime(1300_000);
    await store.cleanSelected();
    expect(tauriCreatePlan).not.toHaveBeenCalled();
    expect(tauriExecuteClean).not.toHaveBeenCalled();
    expect(store.error).toContain('Scan again');
  });

  it('invalidates a cached scan after backend restart without losing historical data', async () => {
    const store = await loaded();
    vi.mocked(tauriGetLastScan).mockResolvedValue(null);
    await store.init();
    expect(store.canClean).toBe(false);
    expect(store.selectedCount).toBe(0);
    expect(store.lastScan?.scan_id).toBe('scan');
  });

  it('coalesces scans, discards old IDs, and does not execute deletion after refresh', async () => {
    const store = await loaded();
    const response = deferred<ScanResult>();
    vi.mocked(tauriScan).mockReturnValue(response.promise);
    const first = store.runScan();
    const second = store.runScan();
    expect(first).toBe(second);
    expect(store.freshness).toBe('refreshing');
    expect(store.selectedCount).toBe(0);
    response.resolve(fixture('new'));
    await second;
    expect(tauriScan).toHaveBeenCalledTimes(1);
    expect(store.selectedMap).toEqual({ 'new-item': true });
    expect(tauriExecuteClean).not.toHaveBeenCalled();
  });

  it('does not let a delayed cache response replace a newer scan', async () => {
    const store = new ScanStore();
    const cache = deferred<ScanResult | null>();
    vi.mocked(tauriGetLastScan).mockReturnValue(cache.promise);
    const init = store.init();
    vi.mocked(tauriScan).mockResolvedValue(fixture('new'));
    await store.runScan();
    cache.resolve(fixture('old'));
    await init;
    expect(store.lastScan?.scan_id).toBe('new');
  });

  it('preserves stale historical results when refresh fails', async () => {
    const store = await loaded();
    vi.mocked(tauriScan).mockRejectedValue(new Error('Scan worker failed'));
    await store.runScan();
    expect(store.freshness).toBe('failed');
    expect(store.canClean).toBe(false);
    expect(store.lastScan?.scan_id).toBe('scan');
    expect(store.selectedCount).toBe(0);
  });

  it('rejects expiry while a plan is being prepared without executing it', async () => {
    const store = await loaded();
    vi.mocked(tauriCreatePlan).mockImplementation(async () => {
      vi.setSystemTime(1300_000);
      return { id: 'plan', targets: [], expected_reclaim_bytes: 10, expires_at: 1600, risk: {
        safe_count: 1, rebuild_count: 0, manual_count: 0, safe_bytes: 10, rebuild_bytes: 0, manual_bytes: 0,
      } };
    });
    await store.cleanSelected();
    expect(tauriExecuteClean).not.toHaveBeenCalled();
    expect(store.error).toContain('expired');
  });

  it.each(['Selected target no longer exists', 'Permission denied', 'Delete plan expired'])('provides a non-destructive recovery for %s', async (message) => {
    const store = await loaded();
    vi.mocked(tauriCreatePlan).mockRejectedValue(new Error(message));
    await store.cleanSelected();
    expect(store.error).toContain(message);
    expect(store.error).toContain('Scan again');
    expect(store.canClean).toBe(false);
    expect(tauriExecuteClean).not.toHaveBeenCalled();
    expect(tauriScan).not.toHaveBeenCalled();
  });

  it('owns one visible timer, auto-rescans stale results, and disposes idempotently', async () => {
    const page = Object.assign(new EventTarget(), { visibilityState: 'visible' });
    const windowEvents = new EventTarget();
    vi.stubGlobal('document', page);
    vi.stubGlobal('window', windowEvents);
    vi.mocked(tauriScan).mockImplementation(async () => fixture('new', Math.floor(Date.now() / 1000)));
    const store = await loaded();
    const first = store.observeFreshness();
    const second = store.observeFreshness();
    await store.init();
    expect(vi.getTimerCount()).toBe(1);
    // The TTL (300s) lapses while visible: the timer rescans automatically
    // instead of leaving a stale warning for a manual click.
    await vi.advanceTimersByTimeAsync(300_000);
    expect(tauriScan).toHaveBeenCalledTimes(1);
    expect(store.lastScan?.scan_id).toBe('new');
    expect(store.freshness).toBe('fresh');
    expect(store.lastScanTrigger).toBe('auto');
    // Fresh results are not rescanned.
    await vi.advanceTimersByTimeAsync(60_000);
    expect(tauriScan).toHaveBeenCalledTimes(1);
    // Hidden panels never scan.
    page.visibilityState = 'hidden';
    page.dispatchEvent(new Event('visibilitychange'));
    expect(vi.getTimerCount()).toBe(0);
    vi.setSystemTime(1000_000 + 600_000);
    store.updateFreshness();
    await vi.advanceTimersByTimeAsync(300_000);
    expect(tauriScan).toHaveBeenCalledTimes(1);
    // Resume revalidates via the cached scan without a hidden tick.
    page.visibilityState = 'visible';
    page.dispatchEvent(new Event('visibilitychange'));
    await store.init();
    expect(store.canClean).toBe(false);
    first(); first();
    expect(vi.getTimerCount()).toBe(1);
    second();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('leaves failed scans for an explicit manual retry', async () => {
    const page = Object.assign(new EventTarget(), { visibilityState: 'visible' });
    const windowEvents = new EventTarget();
    vi.stubGlobal('document', page);
    vi.stubGlobal('window', windowEvents);
    vi.mocked(tauriScan).mockRejectedValue(new Error('Scan worker failed'));
    const store = await loaded();
    const stop = store.observeFreshness();
    await vi.advanceTimersByTimeAsync(300_000);
    expect(tauriScan).toHaveBeenCalledTimes(1);
    expect(store.freshness).toBe('failed');
    await vi.advanceTimersByTimeAsync(300_000);
    expect(tauriScan).toHaveBeenCalledTimes(1);
    stop();
  });

  it('adopts a fresh backend scan from another window instead of rescanning', async () => {
    const page = Object.assign(new EventTarget(), { visibilityState: 'visible' });
    const windowEvents = new EventTarget();
    vi.stubGlobal('document', page);
    vi.stubGlobal('window', windowEvents);
    const store = await loaded();
    const stop = store.observeFreshness();
    // Another visible window finishes a scan while this surface stays open.
    vi.mocked(tauriGetLastScan).mockResolvedValue(fixture('other', 1250));
    await vi.advanceTimersByTimeAsync(300_000);
    expect(tauriScan).not.toHaveBeenCalled();
    expect(store.lastScan?.scan_id).toBe('other');
    expect(store.freshness).toBe('fresh');
    expect(store.lastScanTrigger).toBeNull();
    stop();
  });

  it.each(['hidden', 'disposed'])('does not rescan after becoming %s during revalidation', async (reason) => {
    const page = Object.assign(new EventTarget(), { visibilityState: 'visible' });
    vi.stubGlobal('document', page);
    vi.stubGlobal('window', new EventTarget());
    const store = await loaded();
    const stop = store.observeFreshness();
    await store.init();
    const cache = deferred<ScanResult | null>();
    vi.mocked(tauriGetLastScan).mockReturnValue(cache.promise);
    await vi.advanceTimersByTimeAsync(300_000);
    if (reason === 'hidden') {
      page.visibilityState = 'hidden';
      page.dispatchEvent(new Event('visibilitychange'));
    } else {
      stop();
    }
    cache.resolve(null);
    await vi.advanceTimersByTimeAsync(0);
    expect(tauriScan).not.toHaveBeenCalled();
    stop();
  });

  it('coalesces slow cache revalidation across timer ticks and retries after failure', async () => {
    vi.stubGlobal('document', Object.assign(new EventTarget(), { visibilityState: 'visible' }));
    vi.stubGlobal('window', new EventTarget());
    const store = await loaded();
    const stop = store.observeFreshness();
    await store.init();
    vi.mocked(tauriGetLastScan).mockClear();
    const cache = deferred<ScanResult | null>();
    vi.mocked(tauriGetLastScan).mockReturnValue(cache.promise);
    vi.mocked(tauriScan).mockImplementation(async () => fixture('new', Math.floor(Date.now() / 1000)));
    await vi.advanceTimersByTimeAsync(305_000);
    expect(tauriGetLastScan).toHaveBeenCalledTimes(1);
    cache.resolve(null);
    await vi.advanceTimersByTimeAsync(0);
    expect(tauriScan).toHaveBeenCalledTimes(1);
    vi.mocked(tauriGetLastScan).mockRejectedValue(new Error('Cache unavailable'));
    await vi.advanceTimersByTimeAsync(300_000);
    expect(tauriScan).toHaveBeenCalledTimes(2);
    stop();
  });

  it('does not start a second scan when one begins during cache revalidation', async () => {
    const page = Object.assign(new EventTarget(), { visibilityState: 'visible' });
    const windowEvents = new EventTarget();
    vi.stubGlobal('document', page);
    vi.stubGlobal('window', windowEvents);
    const store = await loaded();
    const cache = deferred<ScanResult | null>();
    vi.mocked(tauriGetLastScan).mockReturnValue(cache.promise);
    vi.mocked(tauriScan).mockImplementation(async () => fixture('new', Math.floor(Date.now() / 1000)));
    const stop = store.observeFreshness();
    await vi.advanceTimersByTimeAsync(300_000);
    expect(tauriScan).not.toHaveBeenCalled();
    await store.runScan();
    cache.resolve(fixture('old'));
    await vi.advanceTimersByTimeAsync(0);
    expect(tauriScan).toHaveBeenCalledTimes(1);
    expect(store.lastScan?.scan_id).toBe('new');
    expect(store.lastScanTrigger).toBe('manual');
    stop();
  });

  it('identifies partial scans, allows manual cleanup, but excludes partial/unavailable items from auto-select', async () => {
    const store = new ScanStore();
    const partialScan: ScanResult = {
      scan_id: 'partial-scan',
      valid_for_seconds: 300,
      started_at: 999,
      finished_at: 1000,
      total_bytes: 50,
      safe_bytes: 50,
      rebuild_bytes: 0,
      manual_bytes: 0,
      quality: 'partial',
      incomplete_reasons: ['Subtree could not be fully read'],
      categories: [{
        category: 'developer',
        display_name: 'Developer',
        total_bytes: 50,
        safe_bytes: 50,
        rebuild_bytes: 0,
        manual_bytes: 0,
        quality: 'partial',
        items: [
          {
            id: 'fresh-safe-item',
            signature_id: 'fresh.sig',
            name: 'Fresh Item',
            category: 'developer',
            risk: 'safe',
            path: '/fresh',
            size: { logical: 30, allocated: 30 },
            file_count: 3,
            description: 'Fully inspected',
            is_selected: false,
            last_modified: null,
            exists: true,
            quality: 'fresh',
            incomplete_reason: null,
          },
          {
            id: 'partial-safe-item',
            signature_id: 'partial.sig',
            name: 'Partial Item',
            category: 'developer',
            risk: 'safe',
            path: '/partial',
            size: { logical: 20, allocated: 20 },
            file_count: 2,
            description: 'Partially inspected',
            is_selected: false,
            last_modified: null,
            exists: true,
            quality: 'partial',
            incomplete_reason: 'Some files inaccessible',
          },
          {
            id: 'unavailable-item',
            signature_id: 'unavail.sig',
            name: 'Unavailable Item',
            category: 'developer',
            risk: 'safe',
            path: '/unavailable',
            size: { logical: 0, allocated: 0 },
            file_count: 0,
            description: 'Cannot read',
            is_selected: false,
            last_modified: null,
            exists: true,
            quality: 'unavailable',
            incomplete_reason: 'Permission denied',
          },
        ],
      }],
    };

    vi.mocked(tauriGetLastScan).mockResolvedValue(partialScan);
    await store.init();

    // Freshness reports 'partial'
    expect(store.freshness).toBe('partial');
    // Manual review cleanup is permitted on partial scan
    expect(store.canClean).toBe(true);

    // Auto-selection only selects fresh items (partial and unavailable are NOT auto-selected)
    expect(store.selectedMap['fresh-safe-item']).toBe(true);
    expect(store.selectedMap['partial-safe-item']).toBe(false);
    expect(store.selectedMap['unavailable-item']).toBe(false);

    // Unavailable item CANNOT be selected even manually
    store.setItemSelected('unavailable-item', true);
    expect(store.selectedMap['unavailable-item']).toBe(false);
    store.toggleItem('unavailable-item');
    expect(store.selectedMap['unavailable-item']).toBe(false);

    // Partial item CAN be selected manually for review cleanup
    store.setItemSelected('partial-safe-item', true);
    expect(store.selectedMap['partial-safe-item']).toBe(true);
  });
});
