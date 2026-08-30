import type { AiProviderUsage, AiUsageSnapshot } from '../models/types';
import { tauriConnectOpenRouter, tauriGetAiUsage } from '../utils/tauri';
import { settingsStore } from './settings.svelte';

const PROVIDER_SHELLS: readonly AiProviderUsage[] = [
  providerShell('codex', 'Codex', 'ChatGPT OAuth'),
  providerShell('claude', 'Claude Code', 'Claude.ai OAuth'),
  providerShell('opencode', 'OpenCode', 'Local providers'),
  providerShell('openrouter', 'OpenRouter', 'OAuth PKCE'),
  providerShell('antigravity', 'Antigravity', 'Google OAuth'),
  providerShell('cursor', 'Cursor', 'Cursor account'),
  providerShell('grok', 'Grok Build', 'xAI account'),
];

function providerShell(id: string, name: string, authLabel: string): AiProviderUsage {
  return {
    id,
    name,
    installed: false,
    connected: false,
    auth_label: authLabel,
    status_message: 'Loading usage metadata…',
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

export function projectProviderSlots(
  providers: readonly AiProviderUsage[],
  isLoading: boolean,
  providerIds: readonly string[] = PROVIDER_SHELLS.map((provider) => provider.id)
): AiProviderUsage[] {
  return providerIds
    .map((id) => {
      const provider = providers.find((candidate) => candidate.id === id);
      if (provider || !isLoading) return provider;
      return PROVIDER_SHELLS.find((shell) => shell.id === id);
    })
    .filter((provider): provider is AiProviderUsage => Boolean(provider));
}

class UsageStore {
  snapshot = $state<AiUsageSnapshot | null>(null);
  loadingProviders = $state<string[]>([]);
  isLoading = $state(false);
  error = $state<string | null>(null);
  connectingProvider = $state<string | null>(null);
  private refreshPromise: Promise<void> | null = null;

  get providers(): AiProviderUsage[] {
    return projectProviderSlots(
      this.snapshot?.providers ?? [],
      this.isLoading,
      settingsStore.settings.ai_accounts_quota_providers
    );
  }

  isProviderLoading(id: string): boolean {
    return this.isLoading && this.loadingProviders.includes(id);
  }

  async refresh(force = false) {
    if (this.refreshPromise) return this.refreshPromise;
    this.refreshPromise = this.performRefresh(force);
    try {
      await this.refreshPromise;
    } finally {
      this.refreshPromise = null;
    }
  }

  async refreshIfStale(ttlMs = 60_000) {
    const fetchedAt = (this.snapshot?.fetched_at ?? 0) * 1000;
    const selectedIds = settingsStore.settings.ai_accounts_quota_providers;
    const snapshotMatchesSelection =
      this.snapshot?.providers.length === selectedIds.length &&
      this.snapshot.providers.every((provider, index) => provider.id === selectedIds[index]);
    if (this.snapshot && snapshotMatchesSelection && Date.now() - fetchedAt < ttlMs) return;
    await this.refresh(false);
  }

  private async performRefresh(force: boolean) {
    this.isLoading = true;
    this.error = null;
    this.loadingProviders = [...settingsStore.settings.ai_accounts_quota_providers];
    try {
      this.snapshot = await tauriGetAiUsage(force, (provider) => {
        this.loadingProviders = this.loadingProviders.filter((id) => id !== provider.id);
        if (!this.snapshot) {
          this.snapshot = {
            fetched_at: Math.floor(Date.now() / 1000),
            providers: [provider],
          };
        } else {
          const index = this.snapshot.providers.findIndex((p) => p.id === provider.id);
          if (index >= 0) {
            this.snapshot.providers[index] = provider;
          } else {
            this.snapshot.providers.push(provider);
          }
        }
      });
    } catch (error: any) {
      this.error = error?.toString() || 'Could not load AI usage';
    } finally {
      this.loadingProviders = [];
      this.isLoading = false;
    }
  }

  async connectOpenRouter() {
    this.connectingProvider = 'openrouter';
    this.error = null;
    try {
      await tauriConnectOpenRouter();
      await this.refresh(true);
    } catch (error: any) {
      this.error = error?.toString() || 'OpenRouter sign-in failed';
    } finally {
      this.connectingProvider = null;
    }
  }
}

export const usageStore = new UsageStore();
