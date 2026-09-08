import type { DiskMetrics, DiskVolume, MemoryMetrics } from '../models/types';
import {
  tauriGetDiskVolumes,
  tauriGetMemoryMetrics,
  tauriTerminateProcessGroup,
} from '../utils/tauri';

export class MemoryStore {
  memory = $state<MemoryMetrics | null>(null);
  disk = $state<DiskMetrics | null>(null);
  volumes = $state<DiskVolume[]>([]);
  isLoading = $state(false);
  isDiskLoading = $state(false);
  isPolling = $state(false);
  error = $state<string | null>(null);
  terminating = $state<string | null>(null);
  lastAction = $state<string | null>(null);

  private timer: ReturnType<typeof setInterval> | null = null;
  private subscriberCount = 0;
  private diskRequest: Promise<void> | null = null;
  private memoryRequest: Promise<void> | null = null;

  async refresh() {
    await Promise.all([this.refreshMemory(), this.refreshDisk()]);
  }

  refreshMemory(): Promise<void> {
    if (this.memoryRequest) return this.memoryRequest;
    this.memoryRequest = this.loadMemory().finally(() => {
      this.memoryRequest = null;
    });
    return this.memoryRequest;
  }

  private async loadMemory() {
    this.isLoading = true;
    this.error = null;
    try {
      this.memory = await tauriGetMemoryMetrics();
    } catch (e: any) {
      this.error = e?.toString() || 'Failed to fetch metrics';
    } finally {
      this.isLoading = false;
    }
  }

  refreshDisk(): Promise<void> {
    if (this.diskRequest) return this.diskRequest;
    this.diskRequest = this.loadDisk().finally(() => {
      this.diskRequest = null;
    });
    return this.diskRequest;
  }

  private async loadDisk() {
    this.isDiskLoading = true;
    try {
      const volumes = await tauriGetDiskVolumes();
      const primary = volumes.find((volume) => volume.is_primary);
      this.volumes = volumes;
      this.disk = primary ? {
        mount_point: primary.mount_point,
        total_bytes: primary.total_bytes,
        used_bytes: primary.used_bytes,
        free_bytes: primary.available_bytes,
        available_bytes: primary.available_bytes,
        percent_used: primary.percent_used,
      } : null;
    } catch (e: any) {
      this.error = e?.toString() || 'Failed to fetch disk metrics';
    } finally {
      this.isDiskLoading = false;
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
      await this.refreshMemory();
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
      this.refreshMemory();
      this.timer = globalThis.setInterval(() => {
        this.refreshMemory();
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
