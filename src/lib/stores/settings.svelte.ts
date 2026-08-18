import type { AiProviderId, QuickPanelSection, ZenithSettings } from '../models/types';
import { tauriGetSettings, tauriSaveSettings } from '../utils/tauri';
import { moveOrdered, toggleOrdered } from '../utils/quickPanel';

class SettingsStore {
  settings = $state<ZenithSettings>({
    launch_at_login: false,
    clean_ai_tools: true,
    clean_developer_tools: true,
    clean_docker: true,
    clean_local_models: false,
    include_rebuild_caches: false,
    theme: 'system',
    excluded_signatures: [],
    quick_panel_sections: ['storage', 'cleanup', 'ai_usage', 'categories', 'memory'],
    quick_panel_ai_providers: ['codex', 'claude', 'opencode', 'openrouter', 'antigravity'],
    dashboard_tabs: ['disk', 'storage', 'docker', 'models', 'memory', 'usage', 'awake'],
    awake_rules: [
      {
        id: 'rule.claude',
        app_name: 'Claude Code',
        executable_pattern: 'claude',
        behavior: 'prevent_system_sleep',
        enabled: true,
      },
      {
        id: 'rule.docker',
        app_name: 'Docker Desktop',
        executable_pattern: 'com.docker.backend',
        behavior: 'prevent_system_sleep',
        enabled: false,
      },
      {
        id: 'rule.terminal',
        app_name: 'Terminal / iTerm2 / Ghostty',
        executable_pattern: 'Terminal|iTerm2|ghostty',
        behavior: 'prevent_system_sleep',
        enabled: false,
      },
    ],
  });

  isLoading = $state(false);
  error = $state<string | null>(null);
  private hasLoaded = false;
  private loadPromise: Promise<void> | null = null;
  private saveQueue: Promise<void> = Promise.resolve();

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
      this.settings = await tauriGetSettings();
      this.settings = {
        ...this.settings,
        quick_panel_sections: this.settings.quick_panel_sections ?? ['storage', 'cleanup', 'ai_usage', 'categories', 'memory'],
        quick_panel_ai_providers: this.settings.quick_panel_ai_providers ?? ['codex', 'claude', 'opencode', 'openrouter', 'antigravity'],
        dashboard_tabs: this.settings.dashboard_tabs ?? ['disk', 'storage', 'docker', 'models', 'memory', 'usage', 'awake'],
      };
      this.hasLoaded = true;
      this.applyTheme(this.settings.theme);
    } catch {
      // keep default
    } finally {
      this.isLoading = false;
    }
  }

  async save(partial: Partial<ZenithSettings>) {
    this.settings = { ...this.settings, ...partial };
    const snapshot = structuredClone(this.settings);
    this.error = null;
    this.saveQueue = this.saveQueue.catch(() => undefined).then(() => tauriSaveSettings(snapshot));
    try {
      await this.saveQueue;
      if (partial.theme) this.applyTheme(partial.theme);
    } catch (error: any) {
      this.error = error?.toString() || 'Could not save preferences';
    }
  }

  async toggleDashboardTab(tab: import('../models/types').DashboardTab) {
    const current = this.settings.dashboard_tabs;
    const next = toggleOrdered(current, tab, true);
    await this.save({ dashboard_tabs: next });
  }

  async moveDashboardTab(tab: import('../models/types').DashboardTab, direction: -1 | 1) {
    const next = moveOrdered(this.settings.dashboard_tabs, tab, direction);
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

  async toggleQuickPanelProvider(provider: AiProviderId) {
    const current = this.settings.quick_panel_ai_providers;
    const next = toggleOrdered(current, provider);
    await this.save({ quick_panel_ai_providers: next });
  }

  async moveQuickPanelProvider(provider: AiProviderId, direction: -1 | 1) {
    const next = moveOrdered(this.settings.quick_panel_ai_providers, provider, direction);
    await this.save({ quick_panel_ai_providers: next });
  }

  applyTheme(theme: string) {
    if (typeof document === 'undefined') return;
    const root = document.documentElement;
    if (theme === 'dark') {
      root.classList.add('dark');
    } else if (theme === 'light') {
      root.classList.remove('dark');
    } else {
      // system
      const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
      if (prefersDark) {
        root.classList.add('dark');
      } else {
        root.classList.remove('dark');
      }
    }
  }
}

export const settingsStore = new SettingsStore();
