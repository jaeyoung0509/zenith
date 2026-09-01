import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { mockApi } from '../lib/api/mock';
import type { PlatformCapabilities } from '../lib/models/types';
import { PlatformCapabilitiesStore } from '../lib/stores/platformCapabilities.svelte';

const windowsCapabilities: PlatformCapabilities = {
  platform: 'windows',
  system_actions: { status: 'unavailable', reason: 'Not ported' },
  cleanup: { status: 'unavailable', reason: 'Not ported' },
  large_files: { status: 'unavailable', reason: 'Not ported' },
  developer_artifacts: { status: 'unavailable', reason: 'Not ported' },
  installed_apps: { status: 'unavailable', reason: 'Not ported' },
  app_uninstall: { status: 'unavailable', reason: 'Not ported' },
  memory_metrics: { status: 'read_only', reason: 'Inspection only' },
  process_termination: { status: 'unavailable', reason: 'Not ported' },
  development_ports: { status: 'unavailable', reason: 'Not ported' },
  keep_awake: { status: 'unavailable', reason: 'Not ported' },
  local_models: { status: 'unavailable', reason: 'Not ported' },
  docker: { status: 'read_only', reason: 'Inspection only' },
  ai_integrations: { status: 'unavailable', reason: 'Not ported' },
};

describe('platform capability contract', () => {
  it('keeps the browser preview payload aligned with the native shape', async () => {
    const capabilities = await mockApi.getPlatformCapabilities();

    expect(capabilities.platform).toBe('macos');
    expect(capabilities.cleanup.status).toBe('available');
    expect(capabilities.memory_metrics.status).toBe('available');
    expect(capabilities.process_termination.status).toBe('available');
  });

  it('distinguishes available, read-only, and unavailable actions', async () => {
    const store = new PlatformCapabilitiesStore(async () => windowsCapabilities);

    await store.load();

    expect(store.isAvailable('cleanup')).toBe(false);
    expect(store.isInspectable('memory_metrics')).toBe(true);
    expect(store.isAvailable('memory_metrics')).toBe(false);
    expect(store.feature('cleanup')?.reason).toBe('Not ported');
    expect(store.isInspectable('docker')).toBe(true);
  });

  it('keeps the newest forced refresh when an older request finishes last', async () => {
    const resolvers: Array<(value: PlatformCapabilities) => void> = [];
    const store = new PlatformCapabilitiesStore(
      () => new Promise((resolve) => resolvers.push(resolve))
    );

    const firstLoad = store.load();
    const forcedLoad = store.load(true);

    resolvers[1](windowsCapabilities);
    await forcedLoad;
    resolvers[0]({ ...windowsCapabilities, platform: 'macos' });
    await firstLoad;

    expect(store.capabilities?.platform).toBe('windows');
    expect(store.isLoading).toBe(false);
  });

  it('keeps native surfaces wired to backend capability gates', () => {
    const dashboard = readFileSync(
      new URL('../routes/dashboard/Dashboard.svelte', import.meta.url),
      'utf8'
    );
    const quickPanel = readFileSync(
      new URL('../routes/quick/QuickPanel.svelte', import.meta.url),
      'utf8'
    );

    expect(dashboard).toContain('Loading platform capabilities');
    expect(dashboard).toContain('platformCapabilitiesStore.isAvailable');
    expect(quickPanel).toContain('platformCapabilitiesStore.isInspectable');
    expect(quickPanel).toContain('cleanupCapability?.reason');
  });

  it('grants only the read-only capability query to the Quick Panel', () => {
    const main = JSON.parse(readFileSync(new URL('../../src-tauri/capabilities/main.json', import.meta.url), 'utf8')) as {
      permissions: string[];
    };
    const quick = JSON.parse(readFileSync(new URL('../../src-tauri/capabilities/quick.json', import.meta.url), 'utf8')) as {
      permissions: string[];
    };

    expect(main.permissions).toContain('allow-get-platform-capabilities');
    expect(quick.permissions).toContain('allow-get-platform-capabilities');
    expect(quick.permissions).not.toContain('allow-terminate-process-group');
    expect(quick.permissions).not.toContain('allow-prepare-app-uninstall');
  });
});
