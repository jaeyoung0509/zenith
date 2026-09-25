import { describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { render } from 'svelte/server';
import type { AgentQuickSessionRow, AiProviderId, AiProviderUsage, AiUsageSnapshot, UsageSummary } from '../lib/models/types';
import QuickUsageGauges from '../lib/components/QuickUsageGauges.svelte';
import QuickPanel from '../routes/quick/QuickPanel.svelte';
import { settingsStore } from '../lib/stores/settings.svelte';
import { usageStore } from '../lib/stores/usage.svelte';
import { scanStore } from '../lib/stores/scan.svelte';
import { platformCapabilitiesStore } from '../lib/stores/platformCapabilities.svelte';
import { goldenCapabilitiesByPlatform } from '../lib/models/platformCapabilities';
import {
  handleQuickPanelFocusChanged,
  isAcceleratorPressed,
  isQuickPanelDismissShortcut,
  moveOrdered,
  platformAccelerator,
  projectAiProviders,
  projectQuickAiRows,
  formatQuickProviderUsage,
  formatQuickReset,
  reorderOrdered,
  selectQuickUsageWindows,
  toggleOrdered,
} from '../lib/utils/quickPanel';

describe('quick panel customization', () => {
  it('never removes the final visible section', () => {
    expect(toggleOrdered(['storage'], 'storage', true)).toEqual(['storage']);
  });

  it('adds disabled entries at the end', () => {
    expect(toggleOrdered(['storage'], 'memory', true)).toEqual(['storage', 'memory']);
  });

  it('moves entries without crossing collection bounds', () => {
    expect(moveOrdered(['storage', 'memory'], 'memory', -1)).toEqual(['memory', 'storage']);
    expect(moveOrdered(['storage', 'memory'], 'storage', -1)).toEqual(['storage', 'memory']);
  });

  it('reorders entries with drag-and-drop', () => {
    expect(reorderOrdered(['a', 'b', 'c', 'd'], 'd', 'b')).toEqual(['a', 'd', 'b', 'c']);
    expect(reorderOrdered(['a', 'b', 'c', 'd'], 'a', 'c')).toEqual(['b', 'c', 'a', 'd']);
    expect(reorderOrdered(['a', 'b'], 'a', 'a')).toEqual(['a', 'b']);
    expect(reorderOrdered(['a', 'b'], 'unknown', 'a')).toEqual(['a', 'b']);
  });

  it('recognizes Escape and both platform accelerators as dismiss shortcuts', () => {
    expect(isQuickPanelDismissShortcut('Escape', false)).toBe(true);
    expect(isQuickPanelDismissShortcut('Escape', true)).toBe(true);
    expect(isQuickPanelDismissShortcut('w', true)).toBe(true);
    expect(isQuickPanelDismissShortcut('w', false)).toBe(false);
    expect(isQuickPanelDismissShortcut('q', true)).toBe(false);
  });

  it('derives the dismiss modifier from the platform context', () => {
    const macChord = { metaKey: true, ctrlKey: false };
    const windowsChord = { metaKey: false, ctrlKey: true };

    expect(platformAccelerator('meta')).toBe('meta');
    expect(platformAccelerator('ctrl')).toBe('ctrl');
    // An unknown context keeps the macOS chord rather than guessing.
    expect(platformAccelerator(null)).toBe('meta');

    expect(isAcceleratorPressed(macChord, 'meta')).toBe(true);
    expect(isAcceleratorPressed(macChord, 'ctrl')).toBe(false);
    expect(isAcceleratorPressed(windowsChord, 'ctrl')).toBe(true);
    expect(isAcceleratorPressed(windowsChord, 'meta')).toBe(false);

    // Ctrl+W must dismiss on Windows and must not dismiss on macOS.
    expect(
      isQuickPanelDismissShortcut(
        'w',
        isAcceleratorPressed(windowsChord, platformAccelerator('ctrl'))
      )
    ).toBe(true);
    expect(
      isQuickPanelDismissShortcut(
        'w',
        isAcceleratorPressed(macChord, platformAccelerator('ctrl'))
      )
    ).toBe(false);
  });

  it('supports toggling and ordering the agent_activity section', () => {
    const initial = ['storage', 'cleanup', 'memory'] as const;
    const withAgent = toggleOrdered(initial as any, 'agent_activity', true);
    expect(withAgent).toContain('agent_activity');
    expect(withAgent[withAgent.length - 1]).toBe('agent_activity');

    const removed = toggleOrdered(withAgent, 'agent_activity', false);
    expect(removed).not.toContain('agent_activity');
  });
});

describe('compact AI summary', () => {
  it('combines provider usage and observed sessions under one identity', () => {
    const provider: AiProviderUsage = {
      id: 'codex', name: 'Codex', installed: true, connected: true,
      auth_label: '', status_message: '', support: 'live',
      windows: [{ label: '5h limit', used_percent: 17, resets_at: 1_800_000_000 }],
      summary: { lifetime_tokens: null, last_7d_tokens: null, peak_daily_tokens: null,
        current_streak_days: null, local_sessions: null, local_cost_usd: null,
        usage_usd: null, limit_remaining_usd: null },
      action_url: null,
    };
    const session: AgentQuickSessionRow = {
      session_id: 'session-1', tool_name: 'Codex', project_name: 'Zenith',
      status: 'working', evidence: 'process_observed', elapsed_seconds: 120,
    };
    const rows = projectQuickAiRows([provider], [session]);
    expect(rows).toHaveLength(1);
    expect(rows[0].provider?.id).toBe('codex');
    expect(rows[0].sessions).toHaveLength(1);
    expect(formatQuickProviderUsage(provider, false)).toContain('17% used');
    expect(formatQuickProviderUsage(provider, true)).toBe('Updating usage…');
    expect(formatQuickProviderUsage(provider, false, true)).toBe('Usage out of date');
  });

  it('formats long and invalid reset intervals with explicit units', () => {
    const now = 1_800_000_000;
    expect(formatQuickReset(now + 2 * 3600 + 15 * 60, now)).toBe('Resets in 2h 15m');
    expect(formatQuickReset(now + 4 * 86400 + 20 * 3600, now)).toBe('Resets in 4d 20h');
    expect(formatQuickReset(null, now)).toBe('Reset time unavailable');
    expect(formatQuickReset(Number.NaN, now)).toBe('Reset time unavailable');
    expect(formatQuickReset(now - 1, now)).toBe('Resets soon');
  });
});

describe('quick panel AI provider projection', () => {
  const defaultSummary: UsageSummary = {
    lifetime_tokens: null,
    last_7d_tokens: null,
    peak_daily_tokens: null,
    current_streak_days: null,
    local_sessions: null,
    local_cost_usd: null,
    usage_usd: null,
    limit_remaining_usd: null,
  };

  const createMockProvider = (
    partial: Partial<AiProviderUsage> & { id: string; name: string }
  ): AiProviderUsage => ({
    installed: true,
    connected: true,
    auth_label: '',
    status_message: '',
    support: 'live',
    windows: [],
    summary: { ...defaultSummary, ...(partial.summary ?? {}) },
    action_url: null,
    ...partial,
  });

  const mockSnapshot: AiUsageSnapshot = {
    fetched_at: Date.now(),
    providers: [
      createMockProvider({
        id: 'codex',
        name: 'Codex',
        windows: [{ label: '5h limit', used_percent: 45, resets_at: Date.now() + 3600000 }],
      }),
      createMockProvider({
        id: 'claude',
        name: 'Claude Code',
        connected: false,
      }),
      createMockProvider({
        id: 'opencode',
        name: 'OpenCode',
        summary: { ...defaultSummary, local_sessions: 12 },
      }),
      createMockProvider({
        id: 'openrouter',
        name: 'OpenRouter',
        summary: { ...defaultSummary, usage_usd: 1.45 },
      }),
    ],
  };

  it('returns empty array when zero providers are enabled', () => {
    const result = projectAiProviders([], mockSnapshot.providers);
    expect(result).toEqual([]);
  });

  it('returns exactly one provider when only one is configured', () => {
    const result = projectAiProviders(['codex'], mockSnapshot.providers);
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe('codex');
    expect(result[0].name).toBe('Codex');
  });

  it('preserves configured order and excludes unselected providers', () => {
    const result = projectAiProviders(['openrouter', 'codex'], mockSnapshot.providers);
    expect(result.map((p) => p.id)).toEqual(['openrouter', 'codex']);
  });

  it('handles configured provider ids that do not exist in snapshot safely', () => {
    const result = projectAiProviders(['antigravity', 'claude'], mockSnapshot.providers);
    expect(result.map((p) => p.id)).toEqual(['claude']);
  });

  it('shows every configured provider even with no observed agent session', () => {
    const previousSettings = settingsStore.settings;
    const previousSnapshot = usageStore.snapshot;
    const previousCapabilities = platformCapabilitiesStore.capabilities;
    try {
      settingsStore.settings = {
        ...previousSettings,
        quick_panel_sections: ['agent_activity'],
        quick_panel_ai_providers: ['codex', 'opencode'],
      };
      platformCapabilitiesStore.capabilities = goldenCapabilitiesByPlatform.macos;
      usageStore.snapshot = mockSnapshot;
      const body = render(QuickPanel).body;
      expect(body).toContain('Codex');
      expect(body).toContain('OpenCode');
      expect(body).not.toContain('Claude Code');
      expect(body).toContain('12 local sessions');
      const activitySection = body.match(/<section[^>]*aria-label="Active AI and services"[\s\S]*?<\/section>/)?.[0];
      expect(activitySection).toBeDefined();
      expect(activitySection).not.toContain('<img');
      expect(activitySection).not.toContain('>OP<');
    } finally {
      settingsStore.settings = previousSettings;
      usageStore.snapshot = previousSnapshot;
      platformCapabilitiesStore.capabilities = previousCapabilities;
    }
  });

  it('keeps a long provider identity readable through loading and unavailable states', () => {
    const previousSettings = settingsStore.settings;
    const previousSnapshot = usageStore.snapshot;
    const previousCapabilities = platformCapabilitiesStore.capabilities;
    const previousLoading = usageStore.isLoading;
    const previousLoadingProviders = usageStore.loadingProviders;
    const longName = 'Provider With A Deliberately Long Display Name';
    try {
      settingsStore.settings = {
        ...previousSettings,
        quick_panel_sections: ['agent_activity'],
        quick_panel_ai_providers: ['codex'],
      };
      platformCapabilitiesStore.capabilities = goldenCapabilitiesByPlatform.macos;
      usageStore.snapshot = {
        fetched_at: Math.floor(Date.now() / 1000),
        providers: [{ ...mockSnapshot.providers[0], name: longName, connected: false }],
      };
      usageStore.isLoading = true;
      usageStore.loadingProviders = ['codex'];
      const loadingBody = render(QuickPanel).body;
      expect(loadingBody).toContain(longName);
      expect(loadingBody).toContain('Updating usage…');
      expect(loadingBody.match(new RegExp(longName, 'g'))).toHaveLength(1);

      usageStore.isLoading = false;
      usageStore.loadingProviders = [];
      const unavailableBody = render(QuickPanel).body;
      expect(unavailableBody).toContain('Not connected');
      expect(unavailableBody.match(new RegExp(longName, 'g'))).toHaveLength(1);
    } finally {
      settingsStore.settings = previousSettings;
      usageStore.snapshot = previousSnapshot;
      platformCapabilitiesStore.capabilities = previousCapabilities;
      usageStore.isLoading = previousLoading;
      usageStore.loadingProviders = previousLoadingProviders;
    }
  });

  it('projects antigravity provider when present in snapshot', () => {
    const snapshotWithAntigravity = [
      ...mockSnapshot.providers,
      createMockProvider({
        id: 'antigravity',
        name: 'Antigravity',
        windows: [{ label: 'Gemini · Weekly', used_percent: 21, resets_at: Date.now() + 10000 }],
      }),
    ];
    const result = projectAiProviders(['antigravity', 'codex'], snapshotWithAntigravity);
    expect(result.map((p) => p.id)).toEqual(['antigravity', 'codex']);
    expect(result[0].windows[0].label).toBe('Gemini · Weekly');
  });

  it('projects placeholder entries when isLoading is true and providers are still in flight', () => {
    const result = projectAiProviders(['codex', 'antigravity'], undefined, true);
    expect(result).toHaveLength(2);
    expect(result.map((p) => p.id)).toEqual(['codex', 'antigravity']);
    expect(result[0].name).toBe('Codex');
    expect(result[1].name).toBe('Antigravity');
  });

  it('selects a complete 5-hour and weekly quota pair regardless of source order', () => {
    const weekly = { label: 'Weekly limit', used_percent: 21, resets_at: null };
    const fiveHour = { label: '5 hour limit', used_percent: 45, resets_at: null };

    expect(selectQuickUsageWindows([weekly, fiveHour])).toEqual({ fiveHour, weekly });
    expect(selectQuickUsageWindows([fiveHour])).toBeNull();
  });

  it('gives a selected provider quota pair readable compact columns', () => {
    const gaugeSource = readFileSync(
      new URL('../lib/components/QuickUsageGauges.svelte', import.meta.url),
      'utf8'
    );
    const body = render(QuickUsageGauges, {
      props: { windows: [
        { label: '5h limit', used_percent: 45, resets_at: null },
        { label: 'Weekly limit', used_percent: 21, resets_at: null },
      ], fallback: 'unused' },
    }).body;
    expect(body).toContain('5 hours');
    expect(body).toContain('1 week');
    expect(body).not.toContain('unused');
    expect(gaugeSource).toContain(
      'grid-cols-[repeat(auto-fit,minmax(min(8rem,100%),1fr))]'
    );
    expect(gaugeSource).not.toContain('grid-cols-2');
    expect(gaugeSource).not.toContain('w-48');
    expect(gaugeSource).toContain('whitespace-nowrap');
    expect(gaugeSource).toContain('tabular-nums');
  });

  it('keeps three-digit quota metadata on one line with tabular numeric styling', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-09-10T00:00:00Z'));

    try {
      const nowSeconds = Math.floor(Date.now() / 1000);
      const rendered = render(QuickUsageGauges, {
        props: {
          windows: [
            { label: '5h limit', used_percent: 100, resets_at: nowSeconds + 23 * 3600 },
            { label: 'Weekly limit', used_percent: 100, resets_at: nowSeconds + 6 * 86400 },
          ],
          fallback: 'unused fallback',
        },
      });

      expect(rendered.body).toContain('whitespace-nowrap');
      expect(rendered.body).toContain('tabular-nums');
      expect(rendered.body).toContain('100%');
      expect(rendered.body).toContain('· 23h');
      expect(rendered.body).toContain('· 6d');
      expect(rendered.body).toContain('aria-label="5 hours: 100% used, resets in 23h"');
      expect(rendered.body).toContain('aria-label="1 week: 100% used, resets in 6d"');
      expect(rendered.body).not.toContain('unused fallback');
    } finally {
      vi.useRealTimers();
    }
  });

  it('renders separate 5-hour and weekly gauge bars with accessible values', () => {
    const rendered = render(QuickUsageGauges, {
      props: {
        windows: [
          { label: '5h limit', used_percent: 45, resets_at: null },
          { label: 'Weekly limit', used_percent: 21, resets_at: null },
        ],
        fallback: 'unused fallback',
      },
    });

    expect(rendered.body).toContain('5 hours');
    expect(rendered.body).toContain('1 week');
    expect(rendered.body).toContain('aria-label="5 hours: 45% used"');
    expect(rendered.body).toContain('aria-label="1 week: 21% used"');
    expect(rendered.body).toContain('width: 45%');
    expect(rendered.body).toContain('width: 21%');
    expect(rendered.body).not.toContain('unused fallback');
  });

  it('keeps the compact fallback when both quota windows are not available', () => {
    const rendered = render(QuickUsageGauges, {
      props: {
        windows: [{ label: '5h limit', used_percent: 45, resets_at: null }],
        fallback: '45% used',
      },
    });

    expect(rendered.body).toContain('45% used');
    expect(rendered.body).not.toContain('Usage limit windows');
    expect(rendered.body).toContain('shrink-0 whitespace-nowrap');
    expect(rendered.body).toContain('tabular-nums');
  });

  it('handles focus loss behaviorally by deactivating without hiding the window', () => {
    const activate = vi.fn();
    const deactivate = vi.fn();

    // Focus lost: deactivates without calling window hide or activation
    handleQuickPanelFocusChanged(false, { activate, deactivate });
    expect(deactivate).toHaveBeenCalledTimes(1);
    expect(activate).not.toHaveBeenCalled();

    // Focus gained: activates
    handleQuickPanelFocusChanged(true, { activate, deactivate });
    expect(activate).toHaveBeenCalledTimes(1);
  });
});

describe('quick cleanup state', () => {
  it('offers a new scan instead of cleanup when the inventory is stale', () => {
    const previousSettings = settingsStore.settings;
    const previousScan = scanStore.lastScan;
    const previousCapabilities = platformCapabilitiesStore.capabilities;
    const now = Math.floor(Date.now() / 1000);
    try {
      settingsStore.settings = { ...previousSettings, quick_panel_sections: ['cleanup'] };
      platformCapabilitiesStore.capabilities = goldenCapabilitiesByPlatform.macos;
      scanStore.lastScan = {
        scan_id: 'expired-quick-scan',
        valid_for_seconds: 1,
        started_at: now - 120,
        finished_at: now - 119,
        categories: [],
        total_bytes: 0,
        safe_bytes: 0,
        rebuild_bytes: 0,
        manual_bytes: 0,
        quality: 'fresh',
        incomplete_reasons: [],
      };
      scanStore.updateFreshness();
      const body = render(QuickPanel).body;
      expect(body).toContain('Scan again to verify safe cleanup.');
      expect(body).toContain('Scan Again');
      expect(body).not.toContain('Clean Safe');
    } finally {
      settingsStore.settings = previousSettings;
      scanStore.lastScan = previousScan;
      platformCapabilitiesStore.capabilities = previousCapabilities;
      scanStore.updateFreshness();
    }
  });
});
