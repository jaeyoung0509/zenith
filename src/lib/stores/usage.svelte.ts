import type { AiUsageSnapshot } from '../models/types';
import { tauriConnectOpenRouter, tauriGetAiUsage } from '../utils/tauri';

class UsageStore {
  snapshot = $state<AiUsageSnapshot | null>(null);
  isLoading = $state(false);
  error = $state<string | null>(null);
  connectingProvider = $state<string | null>(null);

  async refresh() {
    if (this.isLoading) return;
    this.isLoading = true;
    this.error = null;
    try {
      this.snapshot = await tauriGetAiUsage();
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
      await this.refresh();
    } catch (error: any) {
      this.error = error?.toString() || 'OpenRouter sign-in failed';
    } finally {
      this.connectingProvider = null;
    }
  }
}

export const usageStore = new UsageStore();
