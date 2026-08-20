import { describe, expect, it } from 'vitest';
import type { AiProviderUsage, AiUsageSnapshot, UsageSummary } from '../lib/models/types';
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
});
