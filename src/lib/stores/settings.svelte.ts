import type { ZenithSettings } from '../models/types';
import { tauriGetSettings, tauriSaveSettings } from '../utils/tauri';

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

  constructor() {
    this.load();
  }

  async load() {
    this.isLoading = true;
    try {
      this.settings = await tauriGetSettings();
      this.applyTheme(this.settings.theme);
    } catch {
      // keep default
    } finally {
      this.isLoading = false;
    }
  }

  async save(partial: Partial<ZenithSettings>) {
    this.settings = { ...this.settings, ...partial };
    try {
      await tauriSaveSettings(this.settings);
      if (partial.theme) {
        this.applyTheme(partial.theme);
      }
    } catch {
      // ignore
    }
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
