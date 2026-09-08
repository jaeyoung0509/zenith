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

  it('owns one visible timer, revalidates on resume, and disposes idempotently', async () => {
    const page = Object.assign(new EventTarget(), { visibilityState: 'visible' });
    const windowEvents = new EventTarget();
    vi.stubGlobal('document', page);
    vi.stubGlobal('window', windowEvents);
    const store = await loaded();
    const first = store.observeFreshness();
    const second = store.observeFreshness();
    await store.init();
    expect(vi.getTimerCount()).toBe(1);
    page.visibilityState = 'hidden';
    page.dispatchEvent(new Event('visibilitychange'));
    expect(vi.getTimerCount()).toBe(0);
    const calls = vi.mocked(tauriGetLastScan).mock.calls.length;
    await vi.advanceTimersByTimeAsync(300_000);
    expect(tauriGetLastScan).toHaveBeenCalledTimes(calls);
    page.visibilityState = 'visible';
    page.dispatchEvent(new Event('visibilitychange'));
    await store.init();
    expect(store.canClean).toBe(false);
    first(); first();
    expect(vi.getTimerCount()).toBe(1);
    second();
    expect(vi.getTimerCount()).toBe(0);
    expect(tauriScan).not.toHaveBeenCalled();
  });
});
