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
    intensive_cleanup: false,
    theme: 'dark',
    excluded_signatures: ['sig1', 'sig2'],
    quick_panel_sections: ['cleanup', 'storage', 'memory'],
    quick_panel_ai_providers: ['codex', 'claude'],
    dashboard_tabs: ['storage', 'memory', 'docker'],
    dashboard_tabs_revision: 1,
    sidebar_collapsed: false,
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
    ai_control: {},
    agent_notifications: {
      enabled: false,
      notify_on_turn_completed: true,
      notify_on_approval_or_input: true,
      notify_on_possibly_inactive: true,
      hide_project_basename: false,
      inactivity_threshold_minutes: 15,
    },
  };

  it('creates an unproxied plain POJO copy of settings', () => {
    const snapshot = serializeSettingsSnapshot(sampleSettings);
    expect(snapshot).toEqual(sampleSettings);
    expect(snapshot).not.toBe(sampleSettings);
    expect(snapshot.awake_rules).not.toBe(sampleSettings.awake_rules);
    expect(snapshot.awake_rules[0]).not.toBe(sampleSettings.awake_rules[0]);
    expect(snapshot.quick_panel_sections).not.toBe(sampleSettings.quick_panel_sections);
    expect(snapshot.ai_control).not.toBe(sampleSettings.ai_control);
  });

  it('deeply clones all properties without mutating the input source', () => {
    const input: ZenithSettings = { ...sampleSettings };
    const snapshot = serializeSettingsSnapshot(input);
    expect(snapshot).toEqual(sampleSettings);
    expect(snapshot).not.toBe(input);
    expect(snapshot.dashboard_tabs).not.toBe(input.dashboard_tabs);
  });

  it('throws an error if null or undefined settings are passed', () => {
    expect(() => serializeSettingsSnapshot(null as any)).toThrow('Cannot serialize null or undefined settings');
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
  let mockSaveSettings: ReturnType<typeof vi.fn>;
  let mockGetSettings: ReturnType<typeof vi.fn>;

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

    mockSaveSettings = vi.fn().mockResolvedValue(null);
    mockGetSettings = vi.fn().mockResolvedValue({
      launch_at_login: false,
      clean_ai_tools: true,
      clean_developer_tools: true,
      clean_docker: true,
      clean_local_models: false,
      include_rebuild_caches: false,
      intensive_cleanup: false,
      theme: 'system',
      excluded_signatures: [],
      quick_panel_sections: ['storage', 'cleanup'],
      quick_panel_ai_providers: ['codex', 'claude'],
      dashboard_tabs: ['storage', 'memory'],
      sidebar_collapsed: false,
      awake_rules: [],
    });

    store = new SettingsStore(mockGetSettings as any, mockSaveSettings as any);
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
    expect(root.classList.contains('dark')).toBe(true);
  });

  it('keeps intensive cleanup opt-in when loading legacy settings', async () => {
    await store.load();
    expect(store.settings.intensive_cleanup).toBe(false);
    expect(store.settings.sidebar_collapsed).toBe(false);
    expect(store.settings.ai_control.autopilot?.keep_awake_for_verified_sessions).toBe(false);
    expect(store.settings.ai_control.autopilot?.notify_on_battery).toBe(false);
  });

  it('persists the sidebar collapse preference with the rest of the settings', async () => {
    await store.load();
    await store.save({ sidebar_collapsed: true });

    expect(store.settings.sidebar_collapsed).toBe(true);
    expect(mockSaveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({ sidebar_collapsed: true })
    );
  });

  it('responds to system theme changes via matchMedia listener', () => {
    const root = mockDocument.documentElement;
    store.applyTheme('system');
    expect(root.classList.contains('dark')).toBe(true);

    if (listeners['change']) {
      listeners['change']({ matches: false } as MediaQueryListEvent);
      expect(root.classList.contains('dark')).toBe(false);

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

  it('handles rapid sequential saves, sends snapshots in order, and retains the final state', async () => {
    await store.load();
    mockSaveSettings.mockClear();

    const save1 = store.save({ clean_docker: false });
    const save2 = store.save({ clean_local_models: true });
    const save3 = store.save({ theme: 'dark' });

    await Promise.all([save1, save2, save3]);

    expect(mockSaveSettings).toHaveBeenCalledTimes(3);

    const call1Snapshot = mockSaveSettings.mock.calls[0][0];
    const call2Snapshot = mockSaveSettings.mock.calls[1][0];
    const call3Snapshot = mockSaveSettings.mock.calls[2][0];

    expect(call1Snapshot.clean_docker).toBe(false);
    expect(call2Snapshot.clean_docker).toBe(false);
    expect(call2Snapshot.clean_local_models).toBe(true);
    expect(call3Snapshot.clean_docker).toBe(false);
    expect(call3Snapshot.clean_local_models).toBe(true);
    expect(call3Snapshot.theme).toBe('dark');

    expect(store.settings.clean_docker).toBe(false);
    expect(store.settings.clean_local_models).toBe(true);
    expect(store.settings.theme).toBe('dark');
    expect(store.error).toBeNull();
  });

  it('rolls back state, reverts theme, populates error, and handles rejection gracefully on latest failure', async () => {
    await store.load();
    expect(store.settings.theme).toBe('system');

    mockSaveSettings.mockRejectedValueOnce(new Error('Disk write failed'));

    // Should resolve gracefully without throwing unhandled event promise rejection
    await expect(store.save({ theme: 'light', clean_docker: false })).resolves.toBeUndefined();

    expect(store.error).toContain('Disk write failed');
    expect(store.settings.theme).toBe('system');
    expect(store.settings.clean_docker).toBe(true);
    // Theme should be restored to system
    expect(mockDocument.documentElement.classList.contains('dark')).toBe(true);
  });

  it('allows a newer queued save to succeed without rollback when an older save fails', async () => {
    await store.load();

    // First save fails, second save succeeds
    mockSaveSettings
      .mockRejectedValueOnce(new Error('Transient IPC error'))
      .mockResolvedValueOnce(null);

    const saveA = store.save({ clean_docker: false });
    const saveB = store.save({ clean_local_models: true });

    await Promise.all([saveA, saveB]);

    // Save B is the latest revision, so its success should stand and error should be null
    expect(store.settings.clean_docker).toBe(false);
    expect(store.settings.clean_local_models).toBe(true);
    expect(store.error).toBeNull();
  });

  it('rolls back to last known persisted snapshot when all queued saves fail', async () => {
    await store.load();

    mockSaveSettings
      .mockRejectedValueOnce(new Error('First failure'))
      .mockRejectedValueOnce(new Error('Second failure'));

    const saveA = store.save({ clean_docker: false });
    const saveB = store.save({ clean_local_models: true });

    await Promise.all([saveA, saveB]);

    expect(store.settings.clean_docker).toBe(true);
    expect(store.settings.clean_local_models).toBe(false);
    expect(store.error).toContain('Second failure');
  });
});

describe('dashboard tab reordering and customization', () => {
  const defaultTabs: DashboardTab[] = ['storage', 'docker', 'models', 'memory', 'development_servers', 'usage', 'awake'];

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
    expect(reordered).toEqual(['memory', 'storage', 'docker', 'models', 'development_servers', 'usage', 'awake']);
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

describe('quick panel AI provider toggling and order preservation', () => {
  it('sequentially disables providers, preserving enabled order and moving disabled to end', async () => {
    const mockSave = vi.fn().mockResolvedValue(null);
    const mockGet = vi.fn().mockResolvedValue({
      launch_at_login: false,
      clean_ai_tools: true,
      clean_developer_tools: true,
      clean_docker: true,
      clean_local_models: false,
      include_rebuild_caches: false,
      intensive_cleanup: false,
      theme: 'system',
      excluded_signatures: [],
      quick_panel_sections: ['storage', 'cleanup'],
      quick_panel_ai_providers: ['codex', 'claude', 'opencode', 'openrouter', 'antigravity'],
      dashboard_tabs: ['storage', 'memory'],
      sidebar_collapsed: false,
      awake_rules: [],
    });

    const testStore = new SettingsStore(mockGet as any, mockSave as any);
    await testStore.load();

    // Disable claude
    await testStore.toggleQuickPanelProvider('claude');
    expect(testStore.settings.quick_panel_ai_providers).toEqual([
      'codex',
      'opencode',
      'openrouter',
      'antigravity',
    ]);

    // Disable opencode
    await testStore.toggleQuickPanelProvider('opencode');
    expect(testStore.settings.quick_panel_ai_providers).toEqual([
      'codex',
      'openrouter',
      'antigravity',
    ]);

    // Disable openrouter
    await testStore.toggleQuickPanelProvider('openrouter');
    expect(testStore.settings.quick_panel_ai_providers).toEqual([
      'codex',
      'antigravity',
    ]);

    // Re-enable claude (should be added at end of enabled list)
    await testStore.toggleQuickPanelProvider('claude');
    expect(testStore.settings.quick_panel_ai_providers).toEqual([
      'codex',
      'antigravity',
      'claude',
    ]);
  });
});
