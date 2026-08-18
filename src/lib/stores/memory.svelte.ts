import type { DiskMetrics, MemoryMetrics } from '../models/types';
import {
  tauriGetDiskMetrics,
  tauriGetMemoryMetrics,
  tauriTerminateProcessGroup,
} from '../utils/tauri';

class MemoryStore {
  memory = $state<MemoryMetrics | null>(null);
  disk = $state<DiskMetrics | null>(null);
  isLoading = $state(false);
  isPolling = $state(false);
  error = $state<string | null>(null);
  terminating = $state<string | null>(null);
  lastAction = $state<string | null>(null);

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

  async terminateProcessGroup(name: string, force: boolean) {
    if (this.terminating) return;
    this.terminating = name;
    this.error = null;
    this.lastAction = null;
    try {
      const count = await tauriTerminateProcessGroup(name, force);
      this.lastAction = `${force ? 'Force quit' : 'Quit'} requested for ${name} (${count} processes).`;
      await new Promise((resolve) => window.setTimeout(resolve, force ? 300 : 900));
      await this.refresh();
    } catch (error: any) {
      this.error = error?.toString() || `Could not terminate ${name}`;
    } finally {
      this.terminating = null;
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
