import type { AiProviderUsage, AiUsageSnapshot, ProviderDescriptor, ProviderId } from '../models/types';
import {
  refusalForPreview,
  tauriConnectOpenRouter,
  tauriDisconnectAiProvider,
  tauriGetAiProviderDescriptors,
  tauriGetAiUsage,
} from '../utils/tauri';
import { settingsStore } from './settings.svelte';

const PROVIDER_SHELLS: readonly AiProviderUsage[] = [
  providerShell('codex', 'Codex', 'ChatGPT OAuth'),
  providerShell('claude', 'Claude Code', 'Claude.ai OAuth'),
  providerShell('opencode', 'OpenCode', 'Local providers'),
  providerShell('openrouter', 'OpenRouter', 'OAuth PKCE'),
  providerShell('antigravity', 'Antigravity', 'Google OAuth'),
  providerShell('cursor', 'Cursor', 'Cursor account'),
  providerShell('grok-build', 'Grok Build', 'xAI account'),
  providerShell('xai-api', 'xAI API', 'API Key'),
  providerShell('openai-api', 'OpenAI API', 'API Key'),
  providerShell('anthropic-api', 'Anthropic API', 'API Key'),
  providerShell('muse-code', 'Muse Code', 'Meta CLI'),
  providerShell('meta-model-api', 'Meta Model API', 'API Key'),
  providerShell('mistral-api', 'Mistral API', 'API Key'),
  providerShell('fireworks-api', 'Fireworks API', 'API Key'),
];

function providerShell(id: ProviderId, name: string, authLabel: string): AiProviderUsage {
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
  providerIds: readonly (ProviderId | string)[] = PROVIDER_SHELLS.map((provider) => provider.id),
  descriptors: readonly ProviderDescriptor[] = []
): AiProviderUsage[] {
  return providerIds
    .map((id) => {
      const canonicalId: ProviderId = (id as string) === 'grok' ? 'grok-build' : (id as ProviderId);
      const provider = providers.find((candidate) => candidate.id === canonicalId || (candidate.id as string) === id);
      if (provider || !isLoading) return provider;
      const desc = descriptors.find((d) => d.id === canonicalId);
      if (desc) {
        return providerShell(
          desc.id,
          desc.display_name,
          desc.credential_kind === 'api_key' ? 'API Key' : 'Account'
        );
      }
      return PROVIDER_SHELLS.find((shell) => shell.id === canonicalId || (shell.id as string) === id);
    })
    .filter((provider): provider is AiProviderUsage => Boolean(provider));
}

export class UsageStore {
  snapshot = $state<AiUsageSnapshot | null>(null);
  descriptors = $state<ProviderDescriptor[]>([]);
  loadingProviders = $state<ProviderId[]>([]);
  isLoading = $state(false);
  error = $state<string | null>(null);
  connectingProvider = $state<string | null>(null);
  private refreshPromise: Promise<void> | null = null;
  private autoRefreshSubscribers = 0;
  private stopAutoRefresh: (() => void) | null = null;

  /**
   * Visible-only auto-refresh (#128): revalidate the TTL cache while a
   * subscriber surface stays open instead of leaving stale usage data.
   * Failed snapshots stay manual until the user retries explicitly.
   */
  observeAutoRefresh(ttlMs = 60_000, intervalMs = 10_000): () => void {
    this.autoRefreshSubscribers++;
    if (this.autoRefreshSubscribers === 1) {
      const tick = () => {
        if (typeof document !== 'undefined' && document.visibilityState !== 'visible') return;
        if (this.error) return;
        void this.refreshIfStale(ttlMs);
      };
      tick();
      const timer = setInterval(tick, intervalMs);
      this.stopAutoRefresh = () => clearInterval(timer);
    }
    let disposed = false;
    return () => {
      if (disposed) return;
      disposed = true;
      if (--this.autoRefreshSubscribers === 0) {
        this.stopAutoRefresh?.();
        this.stopAutoRefresh = null;
      }
    };
  }

  get providers(): AiProviderUsage[] {
    return projectProviderSlots(
      this.snapshot?.providers ?? [],
      this.isLoading,
      settingsStore.settings.ai_accounts_quota_providers,
      this.descriptors
    );
  }

  isProviderLoading(id: ProviderId | string): boolean {
    return this.isLoading && this.loadingProviders.includes(id as ProviderId);
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

  async loadDescriptors(): Promise<ProviderDescriptor[]> {
    if (this.descriptors.length > 0) return this.descriptors;
    try {
      this.descriptors = await tauriGetAiProviderDescriptors();
    } catch {
      // Keep empty if unavailable
    }
    return this.descriptors;
  }

  get quickPanelProviderOptions() {
    return this.descriptors
      .filter((d) => d.supports_quick_panel)
      .map((d) => ({
        id: d.id,
        label: d.display_name,
      }));
  }

  get accountProviderOptions() {
    return this.descriptors.map((d) => ({
      id: d.id,
      label: d.display_name,
      description: d.description,
    }));
  }

  private async performRefresh(force: boolean) {
    this.isLoading = true;
    this.error = null;
    void this.loadDescriptors();
    this.loadingProviders = [...settingsStore.settings.ai_accounts_quota_providers];
    try {
      this.snapshot = await tauriGetAiUsage(force, (provider) => {
        const canonicalId: ProviderId = (provider.id as string) === 'grok' ? 'grok-build' : (provider.id as ProviderId);
        this.loadingProviders = this.loadingProviders.filter((id) => id !== canonicalId);
        const normalizedProvider: AiProviderUsage = {
          ...provider,
          id: canonicalId,
        };
        if (!this.snapshot) {
          this.snapshot = {
            fetched_at: Math.floor(Date.now() / 1000),
            providers: [normalizedProvider],
          };
        } else {
          const index = this.snapshot.providers.findIndex((p) => p.id === canonicalId);
          if (index >= 0) {
            this.snapshot.providers[index] = normalizedProvider;
          } else {
            this.snapshot.providers.push(normalizedProvider);
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

  async disconnectOpenRouter() {
    const refusal = refusalForPreview('Disconnecting a provider');
    if (refusal) {
      this.error = refusal;
      return;
    }
    this.connectingProvider = 'openrouter';
    this.error = null;
    try {
      await tauriDisconnectAiProvider('openrouter');
      await this.refresh(true);
    } catch (error: any) {
      // The credential is removed locally even when provider revocation fails;
      // surface the message and refresh so the UI shows the disconnected state.
      this.error = error?.toString() || 'OpenRouter disconnect failed';
      await this.refresh(true);
    } finally {
      this.connectingProvider = null;
    }
  }
}

export const usageStore = new UsageStore();
