/** @vitest-environment jsdom */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { commands } from '../lib/bindings/tauri';
import { api, isTauri } from '../lib/api';
import { previewPlatform, setPreviewPlatform } from '../lib/api/mocks/previewPlatform';
import { goldenCapabilitiesByPlatform } from '../lib/models/platformCapabilities';
import { goldenPlatformContextByPlatform } from '../lib/models/platformContext';

afterEach(() => {
  clearMocks();
  setPreviewPlatform('macos');
});

/** Removes the injected native bridge so the next call resolves preview data. */
function withoutNativeBridge() {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
}

describe('native command surface', () => {
  it('native_command_surface_reaches_the_ipc_boundary', async () => {
    const invoked: string[] = [];
    mockIPC(async (cmd: string) => {
      invoked.push(cmd);
      if (cmd === 'get_platform_capabilities') {
        return goldenCapabilitiesByPlatform.windows;
      }
      if (cmd === 'get_platform_context') {
        return goldenPlatformContextByPlatform.windows;
      }
      throw new Error(`unexpected command: ${cmd}`);
    });
    expect(isTauri()).toBe(true);

    const capabilities = await commands.getPlatformCapabilities();
    const context = await commands.getPlatformContext();

    expect(invoked).toEqual(['get_platform_capabilities', 'get_platform_context']);
    expect(capabilities).toEqual(goldenCapabilitiesByPlatform.windows);
    expect(capabilities.app_uninstall.status).toBe('unavailable');
    expect(context.platform).toBe('windows');
    expect(context.reveal_label).toBe('Show in File Explorer');
  });
});

describe('preview platform selection', () => {
  it('preview_platform_selection_returns_the_windows_matrix', async () => {
    withoutNativeBridge();
    expect(isTauri()).toBe(false);
    // macOS is the default preview, so the switch below is what changes it.
    expect(previewPlatform()).toBe('macos');
    expect((await api.getPlatformCapabilities()).platform).toBe('macos');

    setPreviewPlatform('windows');

    const capabilities = await api.getPlatformCapabilities();
    expect(capabilities).toEqual(goldenCapabilitiesByPlatform.windows);
    expect(capabilities.cleanup.status).toBe('available');

    // The vocabulary follows the same selection: the preview never shows one
    // platform's nouns next to another platform's matrix.
    const context = await api.getPlatformContext();
    expect(context).toEqual(goldenPlatformContextByPlatform.windows);
    expect(context.primary_accelerator).toBe('ctrl');
    expect(context.trash_label).toBe('Recycle Bin');

    setPreviewPlatform('linux');
    const linux = await api.getPlatformCapabilities();
    expect(linux).toEqual(goldenCapabilitiesByPlatform.linux);
    expect((await api.getPlatformContext()).platform).toBe('linux');
  });
});
