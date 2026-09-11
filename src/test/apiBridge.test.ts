import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MockInstance } from 'vitest';
import { render } from 'svelte/server';
import PreviewModeIndicator from '../lib/components/PreviewModeIndicator.svelte';
import { apiBridgeStore } from '../lib/stores/apiBridge.svelte';

interface BridgeStub {
  location: { protocol: string; hostname: string };
  __TAURI_INTERNALS__?: { invoke: (command: string, args?: unknown) => Promise<unknown> };
}

// The bridge resolution is intentionally exercised at a module boundary, so
// each case loads a fresh copy of the api module after resetting the registry.
async function loadBridge(stub: BridgeStub) {
  vi.stubGlobal('window', stub);
  vi.resetModules();
  return await import('../lib/api/index');
}

const browserStub: BridgeStub = { location: { protocol: 'http:', hostname: '127.0.0.1' } };

describe('native/preview data source resolution', () => {
  let warn: MockInstance;

  beforeEach(() => {
    warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('answers from preview data in a plain browser session without warning', async () => {
    const bridge = await loadBridge({ ...browserStub });

    expect(bridge.isTauri()).toBe(false);
    expect((await bridge.api.getPlatformCapabilities()).platform).toBe('macos');
    expect(bridge.apiBridgeSnapshot()).toMatchObject({ mode: 'preview', notice: null });
    expect(warn).not.toHaveBeenCalled();
  });

  it('switches to native commands when the bridge appears after a preview call', async () => {
    const stub: BridgeStub = { ...browserStub };
    const bridge = await loadBridge(stub);

    expect((await bridge.api.getPlatformCapabilities()).platform).toBe('macos');
    expect(bridge.apiBridgeSnapshot().mode).toBe('preview');

    const invoke = vi.fn(async (_command: string) => ({ platform: 'windows' }));
    stub.__TAURI_INTERNALS__ = { invoke };

    expect((await bridge.api.getPlatformCapabilities()).platform).toBe('windows');
    expect(invoke.mock.calls[0][0]).toBe('get_platform_capabilities');
    expect(bridge.apiBridgeSnapshot().notice).toBe('bridge-appeared-late');
    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0][0])).toContain('native commands');
  });

  it('warns once when a native webview never receives the bridge', async () => {
    const bridge = await loadBridge({
      location: { protocol: 'tauri:', hostname: 'tauri.localhost' },
    });

    expect(bridge.apiBridgeSnapshot().mode).toBe('preview');
    expect(bridge.selectApiImpl('native', 'preview')).toBe('preview');
    expect(bridge.apiBridgeSnapshot().notice).toBe('native-bridge-missing');
    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0][0])).toContain('preview data');

    // A second call must not repeat the warning or flip to native.
    expect(bridge.selectApiImpl('native', 'preview')).toBe('preview');
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it('keeps a healthy native session on native commands without warning', async () => {
    const invoke = vi.fn(async (_command: string) => ({ platform: 'macos' }));
    const bridge = await loadBridge({
      location: { protocol: 'tauri:', hostname: 'tauri.localhost' },
      __TAURI_INTERNALS__: { invoke },
    });

    expect((await bridge.api.getPlatformCapabilities()).platform).toBe('macos');
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(bridge.apiBridgeSnapshot()).toMatchObject({ mode: 'native', notice: null });
    expect(warn).not.toHaveBeenCalled();
  });

  it('shows the preview-mode indicator only while the data source is preview', () => {
    vi.stubGlobal('window', { ...browserStub });

    expect(render(PreviewModeIndicator).body).toContain('preview-mode-indicator');
    expect(render(PreviewModeIndicator).body).toContain('Preview data');

    apiBridgeStore.snapshot = { mode: 'native', notice: null, message: null };
    expect(render(PreviewModeIndicator).body).not.toContain('preview-mode-indicator');
  });
});
