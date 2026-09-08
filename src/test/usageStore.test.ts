import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AiProviderUsage, AiUsageSnapshot } from '../lib/models/types';
import { UsageStore, projectProviderSlots } from '../lib/stores/usage.svelte';
import { tauriGetAiUsage } from '../lib/utils/tauri';

vi.mock('../lib/utils/tauri', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/utils/tauri')>();
  return {
    ...actual,
    tauriGetAiUsage: vi.fn(),
  };
});

function provider(id: string, name: string): AiProviderUsage {
  return {
    id,
    name,
    installed: true,
    connected: false,
    auth_label: 'Account',
    status_message: 'Manual quota check.',
    support: 'manual',
    windows: [],
    summary: {
      lifetime_tokens: null,
      last_7d_tokens: null,
      peak_daily_tokens: null,
      current_streak_days: null,
      local_sessions: null,
      local_cost_usd: null,
      usage_usd: null,
      limit_remaining_usd: null,
    },
    action_url: null,
  };
}

describe('AI Accounts & Quota provider projection', () => {
  it('shows only selected providers in configured order', () => {
    const providers = [provider('codex', 'Codex'), provider('cursor', 'Cursor'), provider('grok', 'Grok Build')];

    expect(projectProviderSlots(providers, false, ['grok', 'cursor']).map((item) => item.id)).toEqual([
      'grok',
      'cursor',
    ]);
  });

  it('creates loading shells only for selected providers', () => {
    const projected = projectProviderSlots([], true, ['cursor', 'grok']);

    expect(projected.map((item) => item.id)).toEqual(['cursor', 'grok']);
    expect(projected.map((item) => item.name)).toEqual(['Cursor', 'Grok Build']);
  });
});

const AUTO_REFRESH_IDS = ['codex', 'claude', 'opencode', 'openrouter', 'antigravity'];

function autoSnapshot(fetchedAt: number): AiUsageSnapshot {
  return {
    fetched_at: fetchedAt,
    providers: AUTO_REFRESH_IDS.map((id) => provider(id, id)),
  };
}

describe('AI usage auto-refresh while visible', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(1000_000);
    vi.resetAllMocks();
    vi.mocked(tauriGetAiUsage).mockImplementation(async () =>
      autoSnapshot(Math.floor(Date.now() / 1000))
    );
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('refreshes once on subscribe when there is no snapshot', async () => {
    const store = new UsageStore();
    const stop = store.observeAutoRefresh();
    await vi.advanceTimersByTimeAsync(0);
    expect(tauriGetAiUsage).toHaveBeenCalledTimes(1);
    expect(store.snapshot?.providers.map((item) => item.id)).toEqual(AUTO_REFRESH_IDS);
    stop();
  });

  it('revalidates when the TTL lapses while visible and stays quiet when fresh', async () => {
    const store = new UsageStore();
    store.snapshot = autoSnapshot(1000);
    const stop = store.observeAutoRefresh();
    await vi.advanceTimersByTimeAsync(0);
    expect(tauriGetAiUsage).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(tauriGetAiUsage).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(tauriGetAiUsage).toHaveBeenCalledTimes(1);
    stop();
  });

  it('never polls while hidden', async () => {
    const page = Object.assign(new EventTarget(), { visibilityState: 'hidden' });
    vi.stubGlobal('document', page);
    const store = new UsageStore();
    store.snapshot = autoSnapshot(500);
    const stop = store.observeAutoRefresh();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(tauriGetAiUsage).not.toHaveBeenCalled();
    page.visibilityState = 'visible';
    await vi.advanceTimersByTimeAsync(10_000);
    expect(tauriGetAiUsage).toHaveBeenCalledTimes(1);
    stop();
  });

  it('leaves failed snapshots for an explicit manual retry', async () => {
    const store = new UsageStore();
    store.snapshot = autoSnapshot(500);
    store.error = 'Could not load AI usage';
    const stop = store.observeAutoRefresh();
    await vi.advanceTimersByTimeAsync(120_000);
    expect(tauriGetAiUsage).not.toHaveBeenCalled();
    stop();
  });

  it('shares one timer across subscribers and stops after the last dispose', async () => {
    const store = new UsageStore();
    store.snapshot = autoSnapshot(1000);
    const first = store.observeAutoRefresh();
    const second = store.observeAutoRefresh();
    expect(vi.getTimerCount()).toBe(1);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(tauriGetAiUsage).toHaveBeenCalledTimes(1);
    first();
    first();
    expect(vi.getTimerCount()).toBe(1);
    second();
    expect(vi.getTimerCount()).toBe(0);
  });
});
