import { describe, expect, it } from 'vitest';
import type { AiProviderUsage } from '../lib/models/types';
import { projectProviderSlots } from '../lib/stores/usage.svelte';

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
