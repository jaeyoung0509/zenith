import type { AiProviderUsage } from '../models/types';

export function toggleOrdered<T>(items: T[], item: T, keepOne = false): T[] {
  if (!items.includes(item)) return [...items, item];
  if (keepOne && items.length === 1) return items;
  return items.filter((candidate) => candidate !== item);
}

export function moveOrdered<T>(items: T[], item: T, direction: -1 | 1): T[] {
  const next = [...items];
  const index = next.indexOf(item);
  const destination = index + direction;
  if (index < 0 || destination < 0 || destination >= next.length) return items;
  [next[index], next[destination]] = [next[destination], next[index]];
  return next;
}

export function reorderOrdered<T>(items: T[], dragged: T, target: T): T[] {
  const next = [...items];
  const from = next.indexOf(dragged);
  const to = next.indexOf(target);
  if (from < 0 || to < 0 || from === to) return items;
  next.splice(from, 1);
  next.splice(to, 0, dragged);
  return next;
}

export function isQuickPanelDismissShortcut(key: string, metaKey: boolean): boolean {
  return key === 'Escape' || (metaKey && key.toLowerCase() === 'w');
}

const KNOWN_PROVIDER_NAMES: Record<string, string> = {
  codex: 'Codex',
  antigravity: 'Antigravity',
  claude: 'Claude Code',
  opencode: 'OpenCode',
  openrouter: 'OpenRouter',
};

export function projectAiProviders(
  configuredIds: readonly string[],
  providers: readonly AiProviderUsage[] | undefined,
  isLoading = false
): AiProviderUsage[] {
  if (!configuredIds.length) return [];
  if (!providers && !isLoading) return [];

  if (isLoading) {
    return configuredIds.map((id) => {
      const existing = providers?.find((provider) => provider.id === id);
      if (existing) return existing;
      return {
        id,
        name: KNOWN_PROVIDER_NAMES[id] || id,
        installed: true,
        connected: false,
        auth_label: '',
        status_message: 'Loading live usage...',
        support: 'live',
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
    });
  }

  return configuredIds
    .map((id) => providers?.find((provider) => provider.id === id))
    .filter((provider): provider is AiProviderUsage => Boolean(provider));
}
