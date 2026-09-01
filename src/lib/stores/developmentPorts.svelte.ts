import {
  tauriListDevelopmentListeners,
  tauriReleaseDevelopmentListener,
} from '../utils/tauri';
import type {
  DevelopmentListener,
  ReleaseMode,
  ReleaseDevelopmentListenerResult,
} from '../models/types';
import { memoryStore } from './memory.svelte';

export class DevelopmentPortsStore {
  listeners = $state<DevelopmentListener[]>([]);
  isLoading = $state<boolean>(false);
  releasingId = $state<string | null>(null);
  error = $state<string | null>(null);
  lastAction = $state<string | null>(null);

  private inFlightRefresh: Promise<void> | null = null;
  private pollInterval: ReturnType<typeof setInterval> | null = null;
  private subscriberCount = 0;

  async refresh(): Promise<void> {
    if (this.inFlightRefresh) {
      return this.inFlightRefresh;
    }

    this.isLoading = true;
    this.inFlightRefresh = (async () => {
      try {
        const data = await tauriListDevelopmentListeners();
        this.listeners = data;
        this.error = null;
      } catch (err: any) {
        this.error = err?.message || String(err);
      } finally {
        this.isLoading = false;
        this.inFlightRefresh = null;
      }
    })();

    return this.inFlightRefresh;
  }

  async release(
    listener: DevelopmentListener,
    mode: ReleaseMode
  ): Promise<ReleaseDevelopmentListenerResult> {
    if (this.releasingId) {
      throw new Error('Another release operation is in progress');
    }

    this.releasingId = listener.id;
    this.error = null;

    try {
      const result = await tauriReleaseDevelopmentListener(listener.id, mode);

      if (result.outcome === 'released') {
        this.lastAction = `Port ${result.port} released — retry just dev.`;
        // Refresh listeners and memory metrics in background
        void Promise.allSettled([this.refresh(), memoryStore.refreshMemory()]);
      } else if (result.outcome === 'still_listening') {
        if (result.listener) {
          const fresh = result.listener;
          this.listeners = replaceDevelopmentListenerLease(
            this.listeners,
            listener.id,
            fresh
          );
        }
      } else if (result.outcome === 'ownership_changed') {
        this.error = 'Port ownership changed; nothing was stopped.';
        void this.refresh();
      }

      return result;
    } catch (err: any) {
      this.error = err?.message || String(err);
      throw err;
    } finally {
      this.releasingId = null;
    }
  }

  clearError(): void {
    this.error = null;
  }

  clearLastAction(): void {
    this.lastAction = null;
  }

  startPolling(intervalMs = 15000): void {
    this.subscriberCount++;
    if (this.subscriberCount === 1) {
      void this.refresh();
      this.pollInterval = setInterval(() => {
        void this.refresh();
      }, intervalMs);
    }
  }

  stopPolling(): void {
    this.subscriberCount = Math.max(0, this.subscriberCount - 1);
    if (this.subscriberCount === 0 && this.pollInterval) {
      clearInterval(this.pollInterval);
      this.pollInterval = null;
    }
  }
}

export function replaceDevelopmentListenerLease(
  listeners: DevelopmentListener[],
  consumedId: string,
  replacement: DevelopmentListener
): DevelopmentListener[] {
  return listeners.map((listener) =>
    listener.id === consumedId ? replacement : listener
  );
}

export function filterDevelopmentListeners(
  listeners: DevelopmentListener[],
  query: string
): DevelopmentListener[] {
  const q = query.trim().toLowerCase();
  if (!q) return listeners;

  return listeners.filter((listener) => {
    if (String(listener.port).includes(q)) return true;
    if (String(listener.pid).includes(q)) return true;
    if (listener.server_name.toLowerCase().includes(q)) return true;
    if (listener.project_name?.toLowerCase().includes(q)) return true;
    if (listener.working_directory?.toLowerCase().includes(q)) return true;
    if (listener.bind_address.toLowerCase().includes(q)) return true;
    return false;
  });
}

export const developmentPortsStore = new DevelopmentPortsStore();
