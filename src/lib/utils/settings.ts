import type { ZenithSettings } from '../models/types';

/**
 * Creates a clean, unproxied POJO snapshot of settings state safe for
 * structured cloning, JSON serialization, and Tauri IPC transfer.
 */
export function serializeSettingsSnapshot(settings: ZenithSettings): ZenithSettings {
  if (!settings) {
    throw new Error('Cannot serialize null or undefined settings');
  }
  return structuredClone(settings);
}
