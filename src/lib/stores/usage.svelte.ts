import type { AiUsageSnapshot } from '../models/types';
import { tauriConnectOpenRouter, tauriGetAiUsage } from '../utils/tauri';

class UsageStore {
  snapshot = $state<AiUsageSnapshot | null>(null);
  isLoading = $state(false);
  error = $state<string | null>(null);
  connectingProvider = $state<string | null>(null);
  private refreshPromise: Promise<void> | null = null;

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
    try {
      this.snapshot = await tauriGetAiUsage(force);
    } catch (error: any) {
      this.error = error?.toString() || 'Could not load AI usage';
    } finally {
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
