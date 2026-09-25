import { afterEach, describe, expect, it, vi } from 'vitest';
import type { BatteryMetrics, CpuMetrics } from '../lib/models/types';
import { SystemMetricsStore } from '../lib/stores/systemMetrics.svelte';

function cpuFixture(overrides: Partial<CpuMetrics> = {}): CpuMetrics {
  return {
    state: 'fresh',
    usage_percent: 12.5,
    sample_interval_ms: 2500,
    sampled_at: 1_700_000_000_000,
    stale_after_ms: 3000,
    cores: 8,
    reason: null,
    ...overrides,
  };
}

function batteryFixture(overrides: Partial<BatteryMetrics> = {}): BatteryMetrics {
  return {
    presence: 'present',
    charge_state: 'discharging',
    percent: 76,
    time_remaining_seconds: 3600,
    power_source: 'battery',
    sampled_at: 1_700_000_000_000,
    reason: null,
    ...overrides,
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe('SystemMetricsStore polling lifecycle', () => {
  it('shares one timer across subscribers and stops when the last one leaves', async () => {
    vi.useFakeTimers();
    const cpu = vi.fn().mockResolvedValue(cpuFixture());
    const battery = vi.fn().mockResolvedValue(batteryFixture());
    const store = new SystemMetricsStore(cpu, battery);

    store.startPolling(1000, 3000);
    store.startPolling(1000, 3000);
    expect(cpu).toHaveBeenCalledTimes(1);

    store.stopPolling();
    await vi.advanceTimersByTimeAsync(1000);
    expect(cpu).toHaveBeenCalledTimes(2);

    store.stopPolling();
    await vi.advanceTimersByTimeAsync(5000);
    expect(cpu).toHaveBeenCalledTimes(2);
    expect(store.isPolling).toBe(false);
  });

  it('reads the battery far more slowly than the CPU', async () => {
    vi.useFakeTimers();
    const cpu = vi.fn().mockResolvedValue(cpuFixture());
    const battery = vi.fn().mockResolvedValue(batteryFixture());
    const store = new SystemMetricsStore(cpu, battery);

    store.startPolling(1000, 5000);
    await vi.advanceTimersByTimeAsync(0);
    expect(battery).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(4000);
    expect(cpu.mock.calls.length).toBeGreaterThanOrEqual(4);
    expect(battery).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(2000);
    expect(battery).toHaveBeenCalledTimes(2);

    store.stopPolling();
  });
});

describe('SystemMetricsStore cpu history', () => {
  it('records one point per real reading and never invents a value', async () => {
    const store = new SystemMetricsStore(
      vi.fn().mockResolvedValue(cpuFixture({ state: 'warmup', usage_percent: null, sampled_at: null })),
      vi.fn().mockResolvedValue(batteryFixture())
    );
    await store.refreshCpu();
    expect(store.cpuHistory).toEqual([]);

    store.reset();
    const readings: CpuMetrics[] = [
      cpuFixture({ sampled_at: 1000, usage_percent: 10 }),
      cpuFixture({ sampled_at: 2000, usage_percent: 20 }),
      // A second call for the same reading must not extend the history.
      cpuFixture({ sampled_at: 2000, usage_percent: 20 }),
      cpuFixture({ state: 'stale', sampled_at: 3000, usage_percent: 30 }),
      // A stale reading carries the last measurement but is not a new sample.
      cpuFixture({ state: 'stale', sampled_at: 3000, usage_percent: 30 }),
    ];
    let index = 0;
    const store2 = new SystemMetricsStore(
      vi.fn().mockImplementation(async () => readings[Math.min(index++, readings.length - 1)]),
      vi.fn().mockResolvedValue(batteryFixture())
    );
    for (let i = 0; i < readings.length; i++) {
      await store2.refreshCpu();
    }

    expect(store2.cpuHistory).toEqual([
      { at: 1000, percent: 10 },
      { at: 2000, percent: 20 },
    ]);
  });

  it('surfaces a failed probe instead of keeping a stale success', async () => {
    const store = new SystemMetricsStore(
      vi.fn().mockRejectedValue(new Error('probe panicked')),
      vi.fn().mockResolvedValue(batteryFixture())
    );
    await store.refreshCpu();
    expect(store.cpuError).toBe('probe panicked');
    expect(store.cpu).toBeNull();
  });

  it('coalesces concurrent refreshes into one reading', async () => {
    const cpu = vi.fn().mockResolvedValue(cpuFixture());
    const store = new SystemMetricsStore(cpu, vi.fn().mockResolvedValue(batteryFixture()));
    await Promise.all([store.refreshCpu(), store.refreshCpu(), store.refreshCpu()]);
    expect(cpu).toHaveBeenCalledTimes(1);
  });
});
