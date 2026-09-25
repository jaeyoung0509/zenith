import type { BatteryMetrics, CpuMetrics } from '../models/types';
import { tauriGetBatteryMetrics, tauriGetCpuMetrics } from '../utils/tauri';

/** One real CPU observation. The short history is built only from these. */
export interface CpuSample {
  /** Unix milliseconds of the reading. */
  at: number;
  /** System-wide busy percentage, 0-100. */
  percent: number;
}

/** ~2.5 minutes of foreground history at the default 2.5 s cadence. */
const HISTORY_LIMIT = 60;

export class SystemMetricsStore {
  cpu = $state<CpuMetrics | null>(null);
  battery = $state<BatteryMetrics | null>(null);
  /** Real samples only: nothing is interpolated across a gap. */
  cpuHistory = $state<CpuSample[]>([]);
  cpuError = $state<string | null>(null);
  batteryError = $state<string | null>(null);
  isPolling = $state(false);

  /** Cancels the shared timer; keeps the handle type out of this contract. */
  private stopTimer: (() => void) | null = null;
  private getCpuFn: typeof tauriGetCpuMetrics;
  private getBatteryFn: typeof tauriGetBatteryMetrics;

  constructor(
    getCpuFn: typeof tauriGetCpuMetrics = tauriGetCpuMetrics,
    getBatteryFn: typeof tauriGetBatteryMetrics = tauriGetBatteryMetrics
  ) {
    this.getCpuFn = getCpuFn;
    this.getBatteryFn = getBatteryFn;
  }
  private subscriberCount = 0;
  private cpuRequest: Promise<void> | null = null;
  private batteryRequest: Promise<void> | null = null;

  async refresh(): Promise<void> {
    await Promise.all([this.refreshCpu(), this.refreshBattery()]);
  }

  refreshCpu(): Promise<void> {
    if (this.cpuRequest) return this.cpuRequest;
    this.cpuRequest = this.loadCpu().finally(() => {
      this.cpuRequest = null;
    });
    return this.cpuRequest;
  }

  private async loadCpu(): Promise<void> {
    try {
      const metrics = await this.getCpuFn();
      this.cpu = metrics;
      this.cpuError = null;
      if (
        metrics.state === 'fresh' &&
        metrics.usage_percent != null &&
        metrics.sampled_at != null
      ) {
        this.recordSample({ at: metrics.sampled_at, percent: metrics.usage_percent });
      }
    } catch (error) {
      this.cpuError = error instanceof Error ? error.message : String(error);
    }
  }

  refreshBattery(): Promise<void> {
    if (this.batteryRequest) return this.batteryRequest;
    this.batteryRequest = this.loadBattery().finally(() => {
      this.batteryRequest = null;
    });
    return this.batteryRequest;
  }

  private async loadBattery(): Promise<void> {
    try {
      this.battery = await this.getBatteryFn();
      this.batteryError = null;
    } catch (error) {
      this.batteryError = error instanceof Error ? error.message : String(error);
    }
  }

  /**
   * Appends one observed sample. A repeated reading (same timestamp) is a
   * refresh of the same fact and does not extend the history.
   */
  private recordSample(sample: CpuSample): void {
    const last = this.cpuHistory[this.cpuHistory.length - 1];
    if (last && last.at === sample.at) {
      return;
    }
    this.cpuHistory = [...this.cpuHistory, sample].slice(-HISTORY_LIMIT);
  }

  /**
   * Reference-counted polling: Overview, Performance, and the Quick Panel share
   * one timer per window, and the timer stops when the last visible consumer
   * leaves. Nothing polls in a hidden window.
   *
   * CPU follows the foreground cadence. Battery charges far more slowly, so it
   * is read on the first tick and then once per `batteryIntervalMs` instead of
   * re-probing the power source at the CPU rate.
   */
  startPolling(intervalMs: number = 2500, batteryIntervalMs: number = 30_000): void {
    this.subscriberCount++;
    if (this.subscriberCount !== 1) return;

    this.isPolling = true;
    void this.refresh();
    let sinceBatteryMs = 0;
    const handle = globalThis.setInterval(() => {
      void this.refreshCpu();
      sinceBatteryMs += intervalMs;
      if (sinceBatteryMs >= batteryIntervalMs) {
        sinceBatteryMs = 0;
        void this.refreshBattery();
      }
    }, intervalMs);
    this.stopTimer = () => globalThis.clearInterval(handle);
  }

  stopPolling(): void {
    this.subscriberCount = Math.max(0, this.subscriberCount - 1);
    if (this.subscriberCount === 0 && this.stopTimer) {
      this.stopTimer();
      this.stopTimer = null;
      this.isPolling = false;
    }
  }

  reset(): void {
    this.stopTimer?.();
    this.stopTimer = null;
    this.subscriberCount = 0;
    this.isPolling = false;
    this.cpu = null;
    this.battery = null;
    this.cpuHistory = [];
    this.cpuError = null;
    this.batteryError = null;
    this.cpuRequest = null;
    this.batteryRequest = null;
  }
}

export const systemMetricsStore = new SystemMetricsStore();
