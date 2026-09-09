import type { DiskMetrics, DiskVolume, MemoryMetrics, MemoryTerminationMode } from '../models/types';
import {
  tauriGetDiskVolumes,
  tauriGetMemoryMetrics,
  tauriTerminateMemoryGroup,
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

  async terminateMemoryGroup(leaseId: string, mode: MemoryTerminationMode, displayName: string) {
    if (this.terminating) return null;
    if (!leaseId) {
      this.error = `Could not terminate ${displayName}: termination snapshot expired; refresh and try again.`;
      return null;
    }
    this.terminating = displayName;
    this.error = null;
    this.lastAction = null;
    try {
      const result = await tauriTerminateMemoryGroup(leaseId, mode);
      if (result.outcome === 'released') {
        this.lastAction = `${mode === 'force' ? 'Force quit' : 'Quit'} requested for ${displayName} (${result.terminated_count} processes).`;
      } else if (result.outcome === 'still_listening') {
        this.lastAction = `${displayName} is still running after a graceful quit. Use Force Quit to stop it.`;
      } else {
        this.error = `${displayName} changed while quitting (process identity changed). Refresh and try again; no signal was sent.`;
      }
      await new Promise((resolve) => window.setTimeout(resolve, mode === 'force' ? 300 : 900));
      await this.refreshMemory();
      return result;
    } catch (error: any) {
      this.error = error?.toString() || `Could not terminate ${displayName}`;
      return null;
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
