/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * Preview mode answers commands from the fixture layer. Every destructive
 * dispatch has to refuse before that layer can report a deletion that never
 * happened, so the assertions below check both halves: the store reports the
 * refusal and the command behind it is never reached.
 */
vi.mock('../lib/utils/tauri', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../lib/utils/tauri')>();
  return {
    ...actual,
    tauriQuickCleanSafe: vi.fn(),
    tauriExecuteClean: vi.fn(),
    tauriPruneDocker: vi.fn(),
    tauriTerminateMemoryGroup: vi.fn(),
    tauriReleaseDevelopmentListener: vi.fn(),
    tauriDeleteLocalModel: vi.fn(),
    tauriGetLocalModels: vi.fn(),
    tauriRequestStopAgentSession: vi.fn(),
    tauriRemoveAgentIntegration: vi.fn(),
    tauriDisconnectAiProvider: vi.fn(),
  };
});

import * as tauriUtils from '../lib/utils/tauri';
import { refusalForPreview } from '../lib/api';
import { scanStore } from '../lib/stores/scan.svelte';
import { dockerStore } from '../lib/stores/docker.svelte';
import { memoryStore } from '../lib/stores/memory.svelte';
import { developmentPortsStore } from '../lib/stores/developmentPorts.svelte';
import { localModelsStore } from '../lib/stores/models.svelte';
import { agentActivityStore } from '../lib/stores/agentActivity.svelte';
import { usageStore } from '../lib/stores/usage.svelte';

const previewWindow = () => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
};

const nativeWindow = () => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = { invoke: vi.fn() };
};

beforeEach(() => {
  previewWindow();
  vi.clearAllMocks();
});

afterEach(() => {
  previewWindow();
});

describe('destructive dispatches in preview mode', () => {
  it('refuses cleaning without reaching the cleanup command', async () => {
    await expect(scanStore.quickCleanSafe()).resolves.toBeNull();
    expect(scanStore.error).toBe(refusalForPreview('Cleaning'));
    expect(tauriUtils.tauriQuickCleanSafe).not.toHaveBeenCalled();

    await expect(scanStore.cleanItems([])).resolves.toBeNull();
    expect(tauriUtils.tauriExecuteClean).not.toHaveBeenCalled();
  });

  it('refuses a Docker prune without reporting reclaimed bytes', async () => {
    await expect(dockerStore.pruneTarget('container.docker.builder')).resolves.toBe(0);
    expect(dockerStore.error).toBe(refusalForPreview('Pruning Docker data'));
    expect(tauriUtils.tauriPruneDocker).not.toHaveBeenCalled();
  });

  it('refuses process termination without reaching the memory command', async () => {
    await expect(memoryStore.terminateMemoryGroup('lease', 'graceful', 'Chrome')).resolves.toBeNull();
    expect(memoryStore.error).toBe(refusalForPreview('Stopping a process'));
    expect(tauriUtils.tauriTerminateMemoryGroup).not.toHaveBeenCalled();
  });

  it('refuses stopping a development server without reaching the release command', async () => {
    const listener = {
      id: 'listener-1',
      port: 5173,
      protocol: 'http',
      process_name: 'vite',
      pid: 4242,
      project: 'zenith',
      command: 'vite',
      started_at: 1,
      memory_bytes: 0,
      cpu_percent: 0,
      classification: 'development',
      can_release: true,
    } as never;

    await expect(developmentPortsStore.release(listener, 'graceful')).rejects.toThrow(
      'Stopping a development server is unavailable while Zenith is showing preview data.'
    );
    expect(developmentPortsStore.error).toBe(refusalForPreview('Stopping a development server'));
    expect(tauriUtils.tauriReleaseDevelopmentListener).not.toHaveBeenCalled();
  });

  it('refuses deleting a local model without reporting success', async () => {
    const model = { id: 'mlx.zenith-probe', name: 'probe', source: 'mlx', path: '/tmp/probe' } as never;

    await expect(localModelsStore.deleteModel(model)).resolves.toBe(false);
    expect(localModelsStore.error).toBe(refusalForPreview('Deleting a local model'));
    expect(tauriUtils.tauriDeleteLocalModel).not.toHaveBeenCalled();
  });

  it('refuses stopping a session and removing an integration', async () => {
    await expect(agentActivityStore.stopSession('session-1', 'lease-1')).rejects.toThrow(
      'Stopping a session is unavailable while Zenith is showing preview data.'
    );
    expect(agentActivityStore.error).toBe(refusalForPreview('Stopping a session'));
    expect(tauriUtils.tauriRequestStopAgentSession).not.toHaveBeenCalled();

    await expect(agentActivityStore.uninstallIntegration('claude')).rejects.toThrow(
      'Removing an agent integration is unavailable while Zenith is showing preview data.'
    );
    expect(tauriUtils.tauriRemoveAgentIntegration).not.toHaveBeenCalled();
  });

  it('refuses disconnecting a provider without touching stored credentials', async () => {
    await usageStore.disconnectOpenRouter();

    expect(usageStore.error).toBe(refusalForPreview('Disconnecting a provider'));
    expect(tauriUtils.tauriDisconnectAiProvider).not.toHaveBeenCalled();
  });
});

describe('native mode', () => {
  it('passes a destructive dispatch through to the command', async () => {
    nativeWindow();
    vi.mocked(tauriUtils.tauriDeleteLocalModel).mockResolvedValue(undefined as never);
    vi.mocked(tauriUtils.tauriGetLocalModels).mockResolvedValue([]);

    expect(refusalForPreview('Deleting a local model')).toBeNull();
    await expect(
      localModelsStore.deleteModel({
        id: 'mlx.zenith-probe',
        name: 'probe',
        source: 'mlx',
        path: '/tmp/probe',
      } as never)
    ).resolves.toBe(true);
    expect(tauriUtils.tauriDeleteLocalModel).toHaveBeenCalledWith('mlx.zenith-probe');
    expect(localModelsStore.error).toBeNull();
  });
});
