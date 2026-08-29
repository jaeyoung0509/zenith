import type { AiProviderUsage, AiUsageSnapshot } from '../models/types';
import { tauriConnectOpenRouter, tauriGetAiUsage } from '../utils/tauri';

class UsageStore {
  snapshot = $state<AiUsageSnapshot | null>(null);
  loadingProviders = $state<string[]>([]);
  isLoading = $state(false);
  error = $state<string | null>(null);
  connectingProvider = $state<string | null>(null);
  private refreshPromise: Promise<void> | null = null;

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
    if (this.snapshot && Date.now() - fetchedAt < ttlMs) return;
    await this.refresh(false);
  }

  private async performRefresh(force: boolean) {
    this.isLoading = true;
    this.error = null;
    this.loadingProviders = ['codex', 'antigravity', 'opencode', 'openrouter', 'claude'];
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
