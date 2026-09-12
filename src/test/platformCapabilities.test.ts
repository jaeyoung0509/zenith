import { afterEach, describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { render } from 'svelte/server';
import { mockApi } from '../lib/api/mock';
import { goldenCapabilitiesByPlatform } from '../lib/models/platformCapabilities';
import type { PlatformCapabilities } from '../lib/models/types';
import {
  PlatformCapabilitiesStore,
  platformCapabilitiesStore,
} from '../lib/stores/platformCapabilities.svelte';
import QuickPanel from '../routes/quick/QuickPanel.svelte';

const goldenWindows = goldenCapabilitiesByPlatform.windows;

afterEach(() => {
  platformCapabilitiesStore.reset();
});

describe('platform capability contract', () => {
  it('keeps the browser preview payload aligned with the generated snapshot', async () => {
    const capabilities = await mockApi.getPlatformCapabilities();

    expect(capabilities).toEqual(goldenCapabilitiesByPlatform.macos);
    expect(capabilities.platform).toBe('macos');
  });

  it('publishes every platform snapshot with the same feature keys', () => {
    const macosKeys = Object.keys(goldenCapabilitiesByPlatform.macos).sort();

    for (const platform of ['macos', 'windows', 'linux', 'other'] as const) {
      const snapshot = goldenCapabilitiesByPlatform[platform];
      expect(snapshot.platform).toBe(platform);
      expect(Object.keys(snapshot).sort()).toEqual(macosKeys);
    }
  });

  it('reports a reason for every unavailable backend capability', () => {
    const windows = goldenWindows;

    expect(windows.installed_apps.status).toBe('unavailable');
    expect(windows.installed_apps.reason).toBeTruthy();
    expect(windows.app_uninstall.status).toBe('unavailable');
    expect(windows.app_uninstall.reason).toBeTruthy();
    expect(windows.cleanup.status).toBe('available');
    expect(windows.system_actions.status).toBe('available');
  });

  it('distinguishes available, read-only, and unavailable actions', async () => {
    const store = new PlatformCapabilitiesStore(async () => ({
      ...goldenWindows,
      cleanup: { status: 'unavailable', reason: 'Not ported' },
      memory_metrics: { status: 'read_only', reason: 'Inspection only' },
      docker: { status: 'read_only', reason: 'Inspection only' },
    }));

    await store.load();

    expect(store.isAvailable('cleanup')).toBe(false);
    expect(store.isInspectable('memory_metrics')).toBe(true);
    expect(store.isAvailable('memory_metrics')).toBe(false);
    expect(store.feature('cleanup')?.reason).toBe('Not ported');
    expect(store.isInspectable('docker')).toBe(true);
  });

  it('exposes a failed query as an error instead of an unsupported platform', async () => {
    let attempts = 0;
    const store = new PlatformCapabilitiesStore(async () => {
      attempts += 1;
      if (attempts === 1) throw new Error('IPC unavailable');
      return goldenWindows;
    });

    await store.load();

    // The failure must stay distinguishable from "this platform has no adapter".
    expect(store.capabilities).toBeNull();
    expect(store.error).toBe('IPC unavailable');
    expect(store.feature('cleanup')).toBeNull();
    // Gates still fail closed while the query is unresolved.
    expect(store.isAvailable('cleanup')).toBe(false);
    expect(store.isInspectable('system_actions')).toBe(false);

    await store.load(true);

    expect(store.error).toBeNull();
    expect(store.capabilities).toEqual(goldenWindows);
    expect(store.isAvailable('cleanup')).toBe(true);
  });

  it('keeps the newest forced refresh when an older request finishes last', async () => {
    // `Promise.withResolvers` is unavailable on the Node version CI runs, so
    // the deferred is created explicitly.
    interface Deferred {
      promise: Promise<PlatformCapabilities>;
      resolve: (value: PlatformCapabilities) => void;
    }
    const pending: Deferred[] = [];
    const store = new PlatformCapabilitiesStore(() => {
      let resolveDeferred!: (value: PlatformCapabilities) => void;
      const promise = new Promise<PlatformCapabilities>((resolve) => {
        resolveDeferred = resolve;
      });
      pending.push({ promise, resolve: resolveDeferred });
      return promise;
    });

    const firstLoad = store.load();
    const forcedLoad = store.load(true);

    // A forced refresh must not be deduplicated behind the in-flight request.
    expect(pending).toHaveLength(2);

    pending[1].resolve(goldenWindows);
    await forcedLoad;
    pending[0].resolve(goldenCapabilitiesByPlatform.macos);
    await firstLoad;

    expect(store.capabilities?.platform).toBe('windows');
    expect(store.isLoading).toBe(false);
  });

  it('renders a retryable failure state instead of an unsupported-platform claim', () => {
    platformCapabilitiesStore.capabilities = null;
    platformCapabilitiesStore.error = 'IPC unavailable';

    const failed = render(QuickPanel);

    expect(failed.body).toContain('Platform capabilities unavailable');
    expect(failed.body).toContain('IPC unavailable');
    expect(failed.body).toContain('Retry');
    expect(failed.body).not.toContain('Cleanup is unavailable on this platform.');

    platformCapabilitiesStore.error = null;
    platformCapabilitiesStore.capabilities = {
      ...goldenWindows,
      cleanup: { status: 'unavailable', reason: 'Cleanup is not ported to Windows yet.' },
    };

    const unsupported = render(QuickPanel);

    expect(unsupported.body).not.toContain('Platform capabilities unavailable');
    expect(unsupported.body).toContain('Cleanup is not ported to Windows yet.');
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
