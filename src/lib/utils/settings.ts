import type { AwakeRule, ZenithSettings } from '../models/types';

/**
 * Converts reactive Svelte settings state into a clean, unproxied POJO snapshot
 * safe for structured cloning, JSON serialization, and Tauri IPC transfer.
 */
export function serializeSettingsSnapshot(settings: ZenithSettings): ZenithSettings {
  return {
    launch_at_login: Boolean(settings.launch_at_login),
    clean_ai_tools: Boolean(settings.clean_ai_tools),
    clean_developer_tools: Boolean(settings.clean_developer_tools),
    clean_docker: Boolean(settings.clean_docker),
    clean_local_models: Boolean(settings.clean_local_models),
    include_rebuild_caches: Boolean(settings.include_rebuild_caches),
    theme: settings.theme ?? 'system',
    excluded_signatures: Array.isArray(settings.excluded_signatures)
      ? [...settings.excluded_signatures]
      : [],
    quick_panel_sections: Array.isArray(settings.quick_panel_sections)
      ? [...settings.quick_panel_sections]
      : ['storage', 'cleanup', 'ai_usage', 'categories', 'memory'],
    quick_panel_ai_providers: Array.isArray(settings.quick_panel_ai_providers)
      ? [...settings.quick_panel_ai_providers]
      : ['codex', 'claude', 'opencode', 'openrouter', 'antigravity'],
    dashboard_tabs: Array.isArray(settings.dashboard_tabs)
      ? [...settings.dashboard_tabs]
      : ['storage', 'docker', 'models', 'memory', 'usage', 'awake'],
    awake_rules: Array.isArray(settings.awake_rules)
      ? settings.awake_rules.map((rule: AwakeRule) => ({
          id: String(rule.id),
          app_name: String(rule.app_name),
          executable_pattern: String(rule.executable_pattern),
          behavior: rule.behavior,
          power_condition: rule.power_condition,
          enabled: Boolean(rule.enabled),
        }))
      : [],
  };
}
