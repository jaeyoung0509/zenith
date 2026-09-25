import type { AgentQuickSessionRow, AiProviderUsage, ProviderId, UsageWindow } from '../models/types';
import { resolveBrandIdentity } from './brandIcons';

export interface QuickUsageWindowPair {
  fiveHour: UsageWindow;
  weekly: UsageWindow;
}

export function selectQuickUsageWindows(
  windows: readonly UsageWindow[]
): QuickUsageWindowPair | null {
  const fiveHour = windows.find((usageWindow) => {
    const label = usageWindow.label.toLowerCase();
    return label.includes('5h') || label.includes('5 hour');
  });
  const weekly = windows.find((usageWindow) =>
    usageWindow.label.toLowerCase().includes('week')
  );

  return fiveHour && weekly ? { fiveHour, weekly } : null;
}

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

export function isQuickPanelDismissShortcut(key: string, acceleratorPressed: boolean): boolean {
  return key === 'Escape' || (acceleratorPressed && key.toLowerCase() === 'w');
}

/**
 * The platform's primary shortcut modifier. Windows and Linux send `ctrl`;
 * macOS sends `meta`. An unknown context keeps the macOS behavior, which is
 * also the only modifier a webview preview can exercise.
 */
export function platformAccelerator(
  primaryAccelerator: 'meta' | 'ctrl' | null | undefined
): 'meta' | 'ctrl' {
  return primaryAccelerator === 'ctrl' ? 'ctrl' : 'meta';
}

export function isAcceleratorPressed(
  event: Pick<KeyboardEvent, 'metaKey' | 'ctrlKey'>,
  accelerator: 'meta' | 'ctrl'
): boolean {
  return accelerator === 'ctrl' ? event.ctrlKey : event.metaKey;
}
export interface QuickPanelFocusActions {
  activate: () => void | Promise<void>;
  deactivate: () => void;
}

export function handleQuickPanelFocusChanged(
  focused: boolean,
  actions: QuickPanelFocusActions
): void {
  if (focused) {
    void actions.activate();
  } else {
    actions.deactivate();
  }
}

const KNOWN_PROVIDER_NAMES: Record<string, string> = {
  codex: 'Codex',
  antigravity: 'Antigravity',
  claude: 'Claude Code',
  opencode: 'OpenCode',
  openrouter: 'OpenRouter',
  cursor: 'Cursor',
  'grok-build': 'Grok Build',
  grok: 'Grok Build',
  'xai-api': 'xAI API',
  'openai-api': 'OpenAI API',
  'anthropic-api': 'Anthropic API',
  'muse-code': 'Muse Code',
  'meta-model-api': 'Meta Model API',
  'mistral-api': 'Mistral API',
  'fireworks-api': 'Fireworks API',
};

export function projectAiProviders(
  configuredIds: readonly (ProviderId | string)[],
  providers: readonly AiProviderUsage[] | undefined,
  isLoading = false
): AiProviderUsage[] {
  if (!configuredIds.length) return [];
  if (!providers && !isLoading) return [];

  if (isLoading) {
    return configuredIds.map((id) => {
      const canonicalId: ProviderId = ((id === 'grok' ? 'grok-build' : id) as ProviderId);
      const existing = providers?.find((provider) => provider.id === canonicalId || (provider.id as string) === id);
      if (existing) return existing;
      return {
        id: canonicalId,
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

export function formatQuickReset(resetsAt: number | null | undefined, now = Math.floor(Date.now() / 1000)): string {
  if (resetsAt == null || !Number.isFinite(resetsAt) || resetsAt <= 0) return 'Reset time unavailable';
  if (resetsAt <= now) return 'Resets soon';
  const remaining = Math.ceil(resetsAt - now);
  if (remaining < 60) return `Resets in ${remaining}s`;
  if (remaining < 3600) return `Resets in ${Math.ceil(remaining / 60)}m`;
  if (remaining < 86400) {
    const hours = Math.floor(remaining / 3600);
    const minutes = Math.floor((remaining % 3600) / 60);
    return `Resets in ${hours}h${minutes ? ` ${minutes}m` : ''}`;
  }
  const days = Math.floor(remaining / 86400);
  const hours = Math.floor((remaining % 86400) / 3600);
  return `Resets in ${days}d${hours ? ` ${hours}h` : ''}`;
}

export function formatQuickProviderUsage(provider: AiProviderUsage, loading: boolean, stale = false): string {
  if (loading) return 'Updating usage…';
  if (stale) return 'Usage out of date';
  if (!provider.installed) return 'Not installed';
  if (!provider.connected) return 'Not connected';

  const window = provider.windows.find((entry) => entry.used_percent != null);
  if (window?.used_percent != null) {
    const percent = Math.round(Math.min(100, Math.max(0, window.used_percent)));
    return `${percent}% used · ${formatQuickReset(window.resets_at)}`;
  }
  if (provider.summary.local_sessions != null) return `${provider.summary.local_sessions} local sessions`;
  if (provider.summary.usage_usd != null) return `$${provider.summary.usage_usd.toFixed(2)} local usage`;
  return provider.support === 'manual' ? 'Manual data' : 'Usage unavailable';
}

export interface QuickAiRow {
  id: string;
  name: string;
  identity: string;
  provider: AiProviderUsage | null;
  sessions: AgentQuickSessionRow[];
}

/** One identity row combines the configured provider with observed sessions. */
export function projectQuickAiRows(
  providers: readonly AiProviderUsage[],
  sessions: readonly AgentQuickSessionRow[]
): QuickAiRow[] {
  const rows: QuickAiRow[] = [];
  const byIdentity = new Map<string, QuickAiRow>();
  for (const provider of providers) {
    const identity = resolveBrandIdentity(provider.id) ?? provider.id.toLowerCase();
    if (byIdentity.has(identity)) continue;
    const row: QuickAiRow = {
      id: `provider-${identity}`,
      name: provider.name,
      identity: provider.id,
      provider,
      sessions: [],
    };
    rows.push(row);
    byIdentity.set(identity, row);
  }
  for (const session of sessions) {
    const identity = resolveBrandIdentity(session.tool_name) ?? session.tool_name.trim().toLowerCase();
    const existing = byIdentity.get(identity);
    if (existing) {
      existing.sessions.push(session);
      continue;
    }
    const row: QuickAiRow = {
      id: `session-${identity}`,
      name: session.tool_name,
      identity: session.tool_name,
      provider: null,
      sessions: [session],
    };
    rows.push(row);
    byIdentity.set(identity, row);
  }
  return rows;
}
