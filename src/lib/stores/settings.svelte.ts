import type { AiProviderId, DashboardTab, QuickPanelSection, ZenithSettings } from '../models/types';
import { tauriGetSettings, tauriSaveSettings } from '../utils/tauri';
import { moveOrdered, reorderOrdered, toggleOrdered } from '../utils/quickPanel';
import { serializeSettingsSnapshot } from '../utils/settings';

export class SettingsStore {
  settings = $state<ZenithSettings>({
    launch_at_login: false,
    clean_ai_tools: true,
    clean_developer_tools: true,
    clean_docker: true,
    clean_local_models: false,
    include_rebuild_caches: false,
    theme: 'system',
    excluded_signatures: [],
    quick_panel_sections: ['cleanup', 'storage', 'memory', 'ai_usage'],
    quick_panel_ai_providers: ['codex', 'claude', 'opencode', 'openrouter', 'antigravity'],
    dashboard_tabs: ['storage', 'docker', 'models', 'memory', 'usage', 'awake'],
    awake_rules: [
      {
        id: 'rule.codex',
        app_name: 'Codex',
        executable_pattern: 'codex',
        behavior: 'prevent_system_sleep',
        power_condition: 'ac_power_only',
        enabled: false,
      },
      {
        id: 'rule.claude',
        app_name: 'Claude Code',
        executable_pattern: 'claude',
        behavior: 'prevent_system_sleep',
        power_condition: 'ac_power_only',
        enabled: false,
      },
      {
        id: 'rule.docker',
        app_name: 'Docker Desktop',
        executable_pattern: 'com.docker.backend',
        behavior: 'prevent_system_sleep',
        power_condition: 'ac_power_only',
        enabled: false,
      },
      {
        id: 'rule.terminal',
        app_name: 'Terminal / iTerm2 / Ghostty',
        executable_pattern: 'Terminal|iTerm2|ghostty',
        behavior: 'prevent_system_sleep',
        power_condition: 'ac_power_only',
        enabled: false,
      },
    ],
  });

  isLoading = $state(false);
  error = $state<string | null>(null);
  private hasLoaded = false;
  private loadPromise: Promise<void> | null = null;
  private saveQueue: Promise<void> = Promise.resolve();
  private persistedSettings: ZenithSettings | null = null;
  private mediaQueryList: MediaQueryList | null = null;
  private mediaQueryListener: ((e: MediaQueryListEvent) => void) | null = null;

  async load(force = false) {
    if (this.hasLoaded && !force) return;
    if (this.loadPromise) return this.loadPromise;
    this.loadPromise = this.performLoad();
    try {
      await this.loadPromise;
    } finally {
      this.loadPromise = null;
    }
  }

  private async performLoad() {
    this.isLoading = true;
    try {
      const fetched = await tauriGetSettings();
      const normalized: ZenithSettings = {
        ...fetched,
        quick_panel_sections: fetched.quick_panel_sections ?? ['storage', 'cleanup', 'ai_usage', 'categories', 'memory'],
        quick_panel_ai_providers: fetched.quick_panel_ai_providers ?? ['codex', 'claude', 'opencode', 'openrouter', 'antigravity'],
        dashboard_tabs: fetched.dashboard_tabs ?? ['storage', 'docker', 'models', 'memory', 'usage', 'awake'],
      };
      this.settings = normalized;
      this.persistedSettings = serializeSettingsSnapshot(normalized);
      this.hasLoaded = true;
      this.applyTheme(this.settings.theme);
    } catch {
      this.persistedSettings = serializeSettingsSnapshot(this.settings);
    } finally {
      this.isLoading = false;
    }
  }

  async save(partial: Partial<ZenithSettings>) {
    const previousSettings = this.settings;
    const previousTheme = previousSettings.theme;
    this.settings = { ...this.settings, ...partial };

    if (partial.theme !== undefined) {
      this.applyTheme(partial.theme);
    }

    let snapshot: ZenithSettings;
    try {
      snapshot = serializeSettingsSnapshot(this.settings);
    } catch (err: any) {
      this.settings = this.persistedSettings ?? previousSettings;
      this.applyTheme(this.settings.theme);
      this.error = err?.toString() || 'Failed to serialize settings';
      return;
    }

    this.error = null;
    const currentSave = this.saveQueue
      .catch(() => undefined)
      .then(async () => {
        await tauriSaveSettings(snapshot);
        this.persistedSettings = snapshot;
      });
    this.saveQueue = currentSave;

    try {
      await currentSave;
    } catch (error: any) {
      this.error = error?.toString() || 'Could not save preferences';
      this.settings = this.persistedSettings ?? previousSettings;
      this.applyTheme(this.settings.theme);
      throw error;
    }
  }

  async toggleDashboardTab(tab: DashboardTab) {
    const current = this.settings.dashboard_tabs;
    const next = toggleOrdered(current, tab, true);
    await this.save({ dashboard_tabs: next });
  }

  async moveDashboardTab(tab: DashboardTab, direction: -1 | 1) {
    const next = moveOrdered(this.settings.dashboard_tabs, tab, direction);
    await this.save({ dashboard_tabs: next });
  }

  async reorderDashboardTabs(dragged: DashboardTab, target: DashboardTab) {
    const next = reorderOrdered(this.settings.dashboard_tabs, dragged, target);
    await this.save({ dashboard_tabs: next });
  }

  async toggleQuickPanelSection(section: QuickPanelSection) {
    const current = this.settings.quick_panel_sections;
    const next = toggleOrdered(current, section, true);
    await this.save({ quick_panel_sections: next });
  }

  async moveQuickPanelSection(section: QuickPanelSection, direction: -1 | 1) {
    const next = moveOrdered(this.settings.quick_panel_sections, section, direction);
    await this.save({ quick_panel_sections: next });
  }

  async reorderQuickPanelSections(dragged: QuickPanelSection, target: QuickPanelSection) {
    const next = reorderOrdered(this.settings.quick_panel_sections, dragged, target);
    await this.save({ quick_panel_sections: next });
  }

  async toggleQuickPanelProvider(provider: AiProviderId) {
    const current = this.settings.quick_panel_ai_providers;
    const next = toggleOrdered(current, provider);
    await this.save({ quick_panel_ai_providers: next });
  }

  async moveQuickPanelProvider(provider: AiProviderId, direction: -1 | 1) {
    const next = moveOrdered(this.settings.quick_panel_ai_providers, provider, direction);
    await this.save({ quick_panel_ai_providers: next });
  }

  async reorderQuickPanelProviders(dragged: AiProviderId, target: AiProviderId) {
    const next = reorderOrdered(this.settings.quick_panel_ai_providers, dragged, target);
    await this.save({ quick_panel_ai_providers: next });
  }

  applyTheme(theme: string) {
    if (typeof document === 'undefined') return;
    const root = document.documentElement;

    this.removeSystemThemeListener();

    if (theme === 'dark') {
      root.classList.add('dark');
    } else if (theme === 'light') {
      root.classList.remove('dark');
    } else {
      // system
      if (typeof window !== 'undefined' && window.matchMedia) {
        this.mediaQueryList = window.matchMedia('(prefers-color-scheme: dark)');
        const updateTheme = (matches: boolean) => {
          if (matches) {
            root.classList.add('dark');
          } else {
            root.classList.remove('dark');
          }
        };

        updateTheme(this.mediaQueryList.matches);

        this.mediaQueryListener = (e: MediaQueryListEvent) => {
          updateTheme(e.matches);
        };

        if (typeof this.mediaQueryList.addEventListener === 'function') {
          this.mediaQueryList.addEventListener('change', this.mediaQueryListener);
        } else if (typeof (this.mediaQueryList as any).addListener === 'function') {
          (this.mediaQueryList as any).addListener(this.mediaQueryListener);
        }
      }
    }
  }

  removeSystemThemeListener() {
    if (this.mediaQueryList && this.mediaQueryListener) {
      if (typeof this.mediaQueryList.removeEventListener === 'function') {
        this.mediaQueryList.removeEventListener('change', this.mediaQueryListener);
      } else if (typeof (this.mediaQueryList as any).removeListener === 'function') {
        (this.mediaQueryList as any).removeListener(this.mediaQueryListener);
      }
      this.mediaQueryList = null;
      this.mediaQueryListener = null;
    }
  }

  cleanup() {
    this.removeSystemThemeListener();
  }
}

export const settingsStore = new SettingsStore();
