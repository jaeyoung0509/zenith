import type { BatteryChargeState, BatteryPresence, CpuSampleState, MemoryPressure } from '../models/types';

/**
 * Presentation copy for the read-only system readings. Every state the backend
 * can report has one label here, so Overview, Performance, and the Quick Panel
 * cannot drift into three different names for the same fact.
 */

export function cpuStateLabel(state: CpuSampleState): string {
  switch (state) {
    case 'fresh':
      return 'Live';
    case 'warmup':
      return 'Warming up';
    case 'stale':
      return 'Paused';
    case 'unavailable':
      return 'Unavailable';
    case 'failed':
      return 'Read failed';
  }
}

/** A short, factual line for a CPU reading that is not a percentage yet. */
export function cpuStateDescription(state: CpuSampleState, reason: string | null): string {
  const detail = reason ? ` ${reason}` : '';
  switch (state) {
    case 'fresh':
      return 'Share of all logical cores busy over the sampling window.';
    case 'warmup':
      return 'Waiting for a second reading to measure the interval.';
    case 'stale':
      return 'No reading since the last sample; the number shown is the last measured one.';
    case 'unavailable':
      return `Zenith has no CPU adapter for this platform.${detail}`;
    case 'failed':
      return `The CPU probe did not answer.${detail}`;
  }
}

export function batteryPresenceLabel(presence: BatteryPresence): string {
  switch (presence) {
    case 'present':
      return 'Battery';
    case 'absent':
      return 'No battery';
    case 'unavailable':
      return 'Battery unavailable';
  }
}

export function batteryChargeStateLabel(state: BatteryChargeState): string {
  switch (state) {
    case 'charging':
      return 'Charging';
    case 'discharging':
      return 'On battery';
    case 'full':
      return 'Fully charged';
    case 'plugged_in_not_charging':
      return 'Plugged in, not charging';
    case 'unknown':
      return 'Charge state unknown';
  }
}

export function memoryPressureLabel(pressure: MemoryPressure): string {
  switch (pressure) {
    case 'normal':
      return 'Low';
    case 'warning':
      return 'Elevated';
    case 'critical':
      return 'Critical';
  }
}

/**
 * Time remaining is only shown when the platform returned an estimate; the
 * unlimited marker some platforms report is filtered out by the backend, so a
 * missing value here means "not supported", not "zero".
 */
export function formatRemainingTime(seconds: number | null | undefined): string | null {
  if (seconds == null || seconds <= 0) return null;
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.round((seconds % 3600) / 60);
  if (hours <= 0) return `${Math.max(minutes, 1)} min remaining`;
  return `${hours} h ${minutes.toString().padStart(2, '0')} min remaining`;
}
