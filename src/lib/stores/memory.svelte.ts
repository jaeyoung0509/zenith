import type { DiskMetrics, MemoryMetrics } from '../models/types';
import { tauriGetDiskMetrics, tauriGetMemoryMetrics } from '../utils/tauri';

class MemoryStore {
  memory = $state<MemoryMetrics | null>(null);
  disk = $state<DiskMetrics | null>(null);
  isLoading = $state(false);
  isPolling = $state(false);
  error = $state<string | null>(null);

  private timer: number | null = null;
  private subscriberCount = 0;

  async refresh() {
    this.isLoading = true;
    this.error = null;
    try {
      const [mem, dsk] = await Promise.all([
        tauriGetMemoryMetrics(),
        tauriGetDiskMetrics(),
      ]);
      this.memory = mem;
      this.disk = dsk;
    } catch (e: any) {
      this.error = e?.toString() || 'Failed to fetch metrics';
    } finally {
      this.isLoading = false;
    }
  }

  startPolling(intervalMs: number = 2500) {
    this.subscriberCount++;
    if (this.subscriberCount === 1) {
      this.isPolling = true;
      this.refresh();
      this.timer = window.setInterval(() => {
        this.refresh();
      }, intervalMs);
    }
  }

  stopPolling() {
    this.subscriberCount = Math.max(0, this.subscriberCount - 1);
    if (this.subscriberCount === 0 && this.timer !== null) {
      clearInterval(this.timer);
      this.timer = null;
      this.isPolling = false;
    }
  }
}

export const memoryStore = new MemoryStore();
