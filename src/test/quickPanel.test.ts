import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { render } from 'svelte/server';
import type { AiProviderId, AiProviderUsage, AiUsageSnapshot, UsageSummary } from '../lib/models/types';
import QuickUsageGauges from '../lib/components/QuickUsageGauges.svelte';
import { isQuickPanelDismissShortcut, moveOrdered, projectAiProviders, reorderOrdered, toggleOrdered } from '../lib/utils/quickPanel';

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

  it('recognizes Escape and Cmd+W as dismiss shortcuts', () => {
    expect(isQuickPanelDismissShortcut('Escape', false)).toBe(true);
    expect(isQuickPanelDismissShortcut('w', true)).toBe(true);
    expect(isQuickPanelDismissShortcut('w', false)).toBe(false);
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

  it('loads and renders every selected provider inside the consolidated agent section', () => {
    const source = readFileSync(
      new URL('../routes/quick/QuickPanel.svelte', import.meta.url),
      'utf8'
    );
    expect(source).toContain("hasSection('ai_usage') || hasSection('agent_activity')");
    expect(source).toContain('{#each selectedProviders as provider (provider.id)}');
    expect(source).not.toContain('providerValue(selectedProviders[0])');
  });

  it('renders selectedProviders outside agentSummary condition so quota is visible without active sessions', () => {
    const source = readFileSync(
      new URL('../routes/quick/QuickPanel.svelte', import.meta.url),
      'utf8'
    );
    const agentSection = source.substring(source.indexOf("section === 'agent_activity'"));
    const providerLoopIndex = agentSection.indexOf('{#each selectedProviders as provider');
    const agentSummaryIndex = agentSection.indexOf('agentSummary && agentSummary.active_count > 0');
    expect(providerLoopIndex).toBeGreaterThan(0);
    expect(agentSummaryIndex).toBeGreaterThan(0);
    // Verify provider loop appears before agentSummary condition and is not nested inside it
    expect(providerLoopIndex).toBeLessThan(agentSummaryIndex);
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

  it('renders spinning loader and the shared quota gauge in QuickPanel.svelte', () => {
    const source = readFileSync(
      new URL('../routes/quick/QuickPanel.svelte', import.meta.url),
      'utf8'
    );
    expect(source).toContain('usageStore.isProviderLoading(provider.id)');
    expect(source).toContain('RotateCw size={11}');
    expect(source.match(/<QuickUsageGauges/g)).toHaveLength(2);
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
  });
});
