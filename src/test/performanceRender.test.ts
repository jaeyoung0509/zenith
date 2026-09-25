import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import CpuPanel from '../lib/components/performance/CpuPanel.svelte';
import BatteryPanel from '../lib/components/performance/BatteryPanel.svelte';
import MemoryPanel from '../lib/components/performance/MemoryPanel.svelte';
import PerformanceView from '../routes/dashboard/PerformanceView.svelte';
import OverviewView from '../routes/dashboard/OverviewView.svelte';
import { platformCapabilitiesStore } from '../lib/stores/platformCapabilities.svelte';
import { goldenCapabilitiesByPlatform } from '../lib/models/platformCapabilities';
import { systemMetricsStore } from '../lib/stores/systemMetrics.svelte';
import { memoryStore } from '../lib/stores/memory.svelte';

afterEach(() => {
  systemMetricsStore.reset();
  memoryStore.memory = null;
  memoryStore.error = null;
  platformCapabilitiesStore.reset();
});

const cpuBase = {
  sample_interval_ms: 2500,
  sampled_at: Date.now(),
  stale_after_ms: 3000,
  cores: 8,
  reason: null,
};

describe('performance readings', () => {
  it.each([
    ['normal', 'Low pressure'],
    ['warning', 'Elevated pressure'],
    ['critical', 'Critical pressure'],
  ] as const)('Overview keeps measured memory distinct from %s pressure', (pressure, label) => {
    memoryStore.memory = {
      total_bytes: 16 * 1024 ** 3, used_bytes: 10 * 1024 ** 3,
      available_bytes: 6 * 1024 ** 3, free_bytes: 1024 ** 3,
      compressed_bytes: 0, swap_used_bytes: 0, swap_total_bytes: 0,
      pressure, timestamp: 1_800_000_000, top_processes: [],
    };
    const body = render(OverviewView).body;
    expect(body).toContain('10 GB');
    expect(body).toContain('16 GB total');
    expect(body).toContain(label);
  });

  it('Overview distinguishes loading, failed, and unavailable memory without inventing usage', () => {
    platformCapabilitiesStore.capabilities = goldenCapabilitiesByPlatform.macos;
    expect(render(OverviewView).body).toContain('Reading memory…');
    memoryStore.error = 'Memory probe failed';
    expect(render(OverviewView).body).toContain('Memory probe failed');
    memoryStore.error = null;
    platformCapabilitiesStore.capabilities = {
      ...goldenCapabilitiesByPlatform.macos,
      memory_metrics: { status: 'unavailable', reason: 'No memory adapter' },
    };
    const body = render(OverviewView).body;
    expect(body).toContain('No memory adapter');
    expect(body).not.toContain('Low pressure');
    expect(body).not.toContain('0 GB');
  });

  it('cpu fresh renders the measured value, window and real history', () => {
    systemMetricsStore.cpu = { ...cpuBase, state: 'fresh', usage_percent: 37.5 };
    systemMetricsStore.cpuHistory = [
      { at: Date.now() - 2500, percent: 20 },
      { at: Date.now(), percent: 40 },
    ];
    const { body } = render(CpuPanel);
    expect(body).toContain('37.5');
    expect(body).toContain('Logical cores');
    expect(body).toContain('2.5 s');
    expect(body).toContain('Live');
    expect(body).toContain('not the sum of per-process values');
    expect(body).toContain('Recorded history');
    expect(body).toContain('2 samples');
  });

  it('cpu warmup states the state instead of 0%', () => {
    systemMetricsStore.cpu = { ...cpuBase, state: 'warmup', usage_percent: null, sampled_at: null, sample_interval_ms: null };
    const { body } = render(CpuPanel);
    expect(body).toContain('Warming up');
    expect(body).not.toContain('0.0%');
    expect(body).not.toContain('>0%');
  });

  it('cpu stale keeps the last real reading and labels it paused', () => {
    systemMetricsStore.cpu = { ...cpuBase, state: 'stale', usage_percent: 12.5 };
    const { body } = render(CpuPanel);
    expect(body).toContain('12.5');
    expect(body).toContain('Paused');
    expect(body).toContain('last measured one');
  });

  it('cpu unavailable and failed carry the backend reason', () => {
    systemMetricsStore.cpu = { ...cpuBase, state: 'unavailable', usage_percent: null, reason: 'no CPU adapter here' };
    const unavailable = render(CpuPanel).body;
    expect(unavailable).toContain('Unavailable');
    expect(unavailable).toContain('no CPU adapter here');

    systemMetricsStore.cpu = { ...cpuBase, state: 'failed', usage_percent: null, reason: 'probe died' };
    const failed = render(CpuPanel).body;
    expect(failed).toContain('Read failed');
    expect(failed).toContain('probe died');
  });

  it('cpu surfaces a store error', () => {
    systemMetricsStore.cpuError = 'IPC exploded';
    const { body } = render(CpuPanel);
    expect(body).toContain('IPC exploded');
    expect(body).toContain('Read failed');
  });

  it('battery present renders percent, state, estimate and source', () => {
    systemMetricsStore.battery = {
      presence: 'present',
      charge_state: 'charging',
      percent: 76,
      time_remaining_seconds: 3 * 3600 + 25 * 60,
      power_source: 'battery',
      sampled_at: Date.now(),
      reason: null,
    };
    const { body } = render(BatteryPanel);
    expect(body).toContain('76');
    expect(body).toContain('Charging');
    expect(body).toContain('3 h 25 min remaining');
    expect(body).toContain('Power source');
    expect(body).toContain('Battery');
  });

  it('battery without a percent says so, and absent/unavailable stay distinct', () => {
    systemMetricsStore.battery = {
      presence: 'present',
      charge_state: 'discharging',
      percent: null,
      time_remaining_seconds: null,
      power_source: 'ac',
      sampled_at: Date.now(),
      reason: null,
    };
    const noPercent = render(BatteryPanel).body;
    expect(noPercent).toContain('Charge level unavailable');
    expect(noPercent).not.toContain('>0%');
    expect(noPercent).toContain('AC power');
    expect(noPercent).not.toContain('remaining');

    systemMetricsStore.battery = {
      presence: 'absent',
      charge_state: 'unknown',
      percent: null,
      time_remaining_seconds: null,
      power_source: 'ac',
      sampled_at: null,
      reason: null,
    };
    expect(render(BatteryPanel).body).toContain('No battery');

    systemMetricsStore.battery = {
      presence: 'unavailable',
      charge_state: 'unknown',
      percent: null,
      time_remaining_seconds: null,
      power_source: 'unknown',
      sampled_at: null,
      reason: 'the platform withheld the battery reading',
    };
    const unavailable = render(BatteryPanel).body;
    expect(unavailable).toContain('Battery unavailable');
    expect(unavailable).toContain('the platform withheld the battery reading');
  });

  it('performance view wires tabs to one panel and seeds the tab', () => {
    const cpuTab = render(PerformanceView).body;
    expect(cpuTab).toContain('Performance');
    // One keyboard tab stop in the strip, plus the focusable panel itself.
    expect(cpuTab.match(/tabindex="0"/g)).toHaveLength(2);
    expect(cpuTab.match(/tabindex="-1"/g)).toHaveLength(2);
    expect(cpuTab.match(/aria-selected="true"/g)).toHaveLength(1);
    expect(cpuTab.match(/aria-controls=/g)).toHaveLength(3);
    expect(cpuTab).toContain('role="tabpanel"');
    expect(cpuTab).toContain('CPU readings');

    memoryStore.memory = {
      total_bytes: 16 * 1024 ** 3,
      used_bytes: 10 * 1024 ** 3,
      available_bytes: 3 * 1024 ** 3,
      free_bytes: 610 * 1024 ** 2,
      compressed_bytes: 2.4 * 1024 ** 3,
      swap_used_bytes: 512 * 1024 ** 2,
      swap_total_bytes: 4 * 1024 ** 3,
      pressure: 'normal',
      timestamp: Math.floor(Date.now() / 1000),
      top_processes: [
        { pid: 1, pids: [1], name: '한국어 개발 앱', memory_bytes: 100 * 1024 ** 3, process_count: 100, can_terminate: true, termination_lease_id: 'l1' },
        { pid: 2, pids: [2], name: 'Protected', memory_bytes: 9 * 1024 ** 2, process_count: 1, can_terminate: false, termination_lease_id: null },
      ],
    };
    const memoryTab = render(PerformanceView, { props: { initialTab: 'memory' } }).body;
    expect(memoryTab).toContain('Memory pressure: Low');
    expect(memoryTab).toContain('한국어 개발 앱');
    expect(memoryTab.match(/w-\[10ch\]/g)).toHaveLength(2);
    expect(memoryTab.match(/w-16 shrink-0/g)).toHaveLength(2);
    expect(memoryTab).toContain('min-h-16');
    expect(memoryTab).not.toContain('backdrop-blur');
    expect(memoryTab).not.toContain('Memory looks healthy');
    expect(memoryTab.toLowerCase()).not.toContain('unknown');

    const batteryTab = render(PerformanceView, { props: { initialTab: 'battery' } }).body;
    expect(batteryTab).toContain('Reading the battery state');
    expect(batteryTab).not.toContain('Memory pressure');
  });
});
