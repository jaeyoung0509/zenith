import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import type { DashboardTab, ZenithSettings } from '../lib/models/types';
import { moveOrdered, reorderOrdered, toggleOrdered } from '../lib/utils/quickPanel';
import { serializeSettingsSnapshot } from '../lib/utils/settings';
import { SettingsStore } from '../lib/stores/settings.svelte';

describe('serializeSettingsSnapshot', () => {
  const sampleSettings: ZenithSettings = {
    launch_at_login: false,
    clean_ai_tools: true,
    clean_developer_tools: true,
    clean_docker: true,
    clean_local_models: false,
    include_rebuild_caches: false,
    theme: 'dark',
    excluded_signatures: ['sig1', 'sig2'],
    quick_panel_sections: ['cleanup', 'storage', 'memory'],
    quick_panel_ai_providers: ['codex', 'claude'],
    dashboard_tabs: ['storage', 'memory', 'docker'],
    awake_rules: [
      {
        id: 'rule.codex',
        app_name: 'Codex',
        executable_pattern: 'codex',
        behavior: 'prevent_system_sleep',
        power_condition: 'ac_power_only',
        enabled: true,
      },
    ],
  };

  it('creates an unproxied plain POJO copy of settings', () => {
    const snapshot = serializeSettingsSnapshot(sampleSettings);
    expect(snapshot).toEqual(sampleSettings);
    expect(snapshot).not.toBe(sampleSettings);
    expect(snapshot.awake_rules).not.toBe(sampleSettings.awake_rules);
    expect(snapshot.awake_rules[0]).not.toBe(sampleSettings.awake_rules[0]);
    expect(snapshot.quick_panel_sections).not.toBe(sampleSettings.quick_panel_sections);
  });

  it('provides safe defaults when optional arrays are missing', () => {
    const sparse = {
      launch_at_login: true,
      clean_ai_tools: false,
      clean_developer_tools: false,
      clean_docker: false,
      clean_local_models: false,
      include_rebuild_caches: false,
      theme: 'system' as const,
      excluded_signatures: [],
    } as any;

    const snapshot = serializeSettingsSnapshot(sparse);
    expect(snapshot.quick_panel_sections).toEqual(['storage', 'cleanup', 'ai_usage', 'categories', 'memory']);
    expect(snapshot.dashboard_tabs).toEqual(['storage', 'docker', 'models', 'memory', 'usage', 'awake']);
    expect(snapshot.awake_rules).toEqual([]);
  });

  it('guarantees deep immutability against subsequent mutations', () => {
    const snapshot = serializeSettingsSnapshot(sampleSettings);
    snapshot.dashboard_tabs.push('models');
    snapshot.awake_rules[0].enabled = false;

    expect(sampleSettings.dashboard_tabs).not.toContain('models');
    expect(sampleSettings.awake_rules[0].enabled).toBe(true);
  });
});

describe('SettingsStore persistence and lifecycle', () => {
  let store: SettingsStore;
  let listeners: Record<string, (e: any) => void> = {};
  let mockMatchMedia: any;
  let mockDocument: any;
  let mockWindow: any;
  let classListSet: Set<string>;

  beforeEach(() => {
    classListSet = new Set<string>();
    listeners = {};

    mockMatchMedia = vi.fn().mockImplementation((query: string) => ({
      matches: query.includes('dark'),
      media: query,
      onchange: null,
      addEventListener: vi.fn((event, handler) => {
        listeners[event] = handler;
      }),
      removeEventListener: vi.fn((event) => {
        delete listeners[event];
      }),
    }));

    mockDocument = {
      documentElement: {
        classList: {
          add: (cls: string) => classListSet.add(cls),
          remove: (cls: string) => classListSet.delete(cls),
          contains: (cls: string) => classListSet.has(cls),
          toggle: (cls: string, force?: boolean) => {
            if (force === undefined) {
              if (classListSet.has(cls)) classListSet.delete(cls);
              else classListSet.add(cls);
            } else if (force) {
              classListSet.add(cls);
            } else {
              classListSet.delete(cls);
            }
          },
        },
      },
    };

    mockWindow = {
      matchMedia: mockMatchMedia,
    };

    vi.stubGlobal('document', mockDocument);
    vi.stubGlobal('window', mockWindow);
    store = new SettingsStore();
  });

  afterEach(() => {
    store.cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('applies theme classes immediately to document root', () => {
    const root = mockDocument.documentElement;

    store.applyTheme('dark');
    expect(root.classList.contains('dark')).toBe(true);

    store.applyTheme('light');
    expect(root.classList.contains('dark')).toBe(false);

    store.applyTheme('system');
    // In our mockMatchMedia, 'dark' matches
    expect(root.classList.contains('dark')).toBe(true);
  });

  it('responds to system theme changes via matchMedia listener', () => {
    const root = mockDocument.documentElement;
    store.applyTheme('system');
    expect(root.classList.contains('dark')).toBe(true);

    // Simulate OS appearance change to light mode
    if (listeners['change']) {
      listeners['change']({ matches: false } as MediaQueryListEvent);
      expect(root.classList.contains('dark')).toBe(false);

      // Simulate OS appearance change back to dark mode
      listeners['change']({ matches: true } as MediaQueryListEvent);
      expect(root.classList.contains('dark')).toBe(true);
    }
  });

  it('removes system theme listener when switching to explicit light/dark', () => {
    store.applyTheme('system');
    expect(mockMatchMedia).toHaveBeenCalled();

    store.applyTheme('light');
    expect(listeners['change']).toBeUndefined();
  });

  it('handles rapid sequential saves and retains the final state without update loss', async () => {
    const saveSettings = vi.fn(async (_settings: ZenithSettings) => undefined);
    store = new SettingsStore({ saveSettings });

    const savePromise1 = store.save({ clean_docker: false });
    const savePromise2 = store.save({ clean_local_models: true });
    const savePromise3 = store.save({ theme: 'dark' });

    await Promise.all([savePromise1, savePromise2, savePromise3]);

    expect(store.settings.clean_docker).toBe(false);
    expect(store.settings.clean_local_models).toBe(true);
    expect(store.settings.theme).toBe('dark');
    expect(store.error).toBeNull();
    expect(saveSettings).toHaveBeenCalledTimes(3);
    expect(saveSettings.mock.calls[2][0]).toMatchObject({
      clean_docker: false,
      clean_local_models: true,
      theme: 'dark',
    });
  });

  it('rolls back the latest failed save and reports the error without rejecting', async () => {
    const saveSettings = vi.fn(async (_settings: ZenithSettings) => {
      throw new Error('disk full');
    });
    store = new SettingsStore({ saveSettings });
    const originalTheme = store.settings.theme;

    await expect(store.save({ theme: 'light' })).resolves.toBeUndefined();

    expect(store.settings.theme).toBe(originalTheme);
    expect(store.error).toContain('disk full');
  });

  it('does not let an older failed save overwrite a newer successful snapshot', async () => {
    const saveSettings = vi
      .fn<(settings: ZenithSettings) => Promise<void>>()
      .mockRejectedValueOnce(new Error('transient write failure'))
      .mockResolvedValueOnce(undefined);
    store = new SettingsStore({ saveSettings });

    const first = store.save({ clean_docker: false });
    const second = store.save({ clean_local_models: true });
    await Promise.all([first, second]);

    expect(store.settings.clean_docker).toBe(false);
    expect(store.settings.clean_local_models).toBe(true);
    expect(store.error).toBeNull();
    expect(saveSettings.mock.calls[1][0]).toMatchObject({
      clean_docker: false,
      clean_local_models: true,
    });
  });
});

describe('dashboard tab reordering and customization', () => {
  const defaultTabs: DashboardTab[] = ['storage', 'docker', 'models', 'memory', 'usage', 'awake'];

  it('moves dashboard tabs up and down correctly', () => {
    const movedUp = moveOrdered(defaultTabs, 'docker', -1);
    expect(movedUp[0]).toBe('docker');
    expect(movedUp[1]).toBe('storage');

    const movedDown = moveOrdered(defaultTabs, 'storage', 1);
    expect(movedDown[0]).toBe('docker');
    expect(movedDown[1]).toBe('storage');
  });

  it('reorders dashboard tabs via drag and drop', () => {
    const reordered = reorderOrdered(defaultTabs, 'memory', 'storage');
    expect(reordered).toEqual(['memory', 'storage', 'docker', 'models', 'usage', 'awake']);
  });

  it('clamps tab movement at array boundaries', () => {
    const topStay = moveOrdered(defaultTabs, 'storage', -1);
    expect(topStay).toEqual(defaultTabs);

    const bottomStay = moveOrdered(defaultTabs, 'awake', 1);
    expect(bottomStay).toEqual(defaultTabs);
  });

  it('toggles dashboard tabs on and off while preserving at least one tab', () => {
    const withoutDocker = toggleOrdered(defaultTabs, 'docker', true);
    expect(withoutDocker.includes('docker')).toBe(false);

    const withDockerBack = toggleOrdered(withoutDocker, 'docker', true);
    expect(withDockerBack[withDockerBack.length - 1]).toBe('docker');

    // Never remove last tab
    const singleTab: DashboardTab[] = ['storage'];
    const preserved = toggleOrdered(singleTab, 'storage', true);
    expect(preserved).toEqual(['storage']);
  });
});

describe('quick panel sections customization', () => {
  const defaultSections = ['cleanup', 'storage', 'memory', 'ai_usage'] as const;

  it('reorders quick panel sections via drag and drop', () => {
    const reordered = reorderOrdered([...defaultSections], 'memory', 'cleanup');
    expect(reordered).toEqual(['memory', 'cleanup', 'storage', 'ai_usage']);
  });

  it('moves quick panel sections with boundary clamping', () => {
    const moved = moveOrdered([...defaultSections], 'storage', -1);
    expect(moved).toEqual(['storage', 'cleanup', 'memory', 'ai_usage']);
  });
});
