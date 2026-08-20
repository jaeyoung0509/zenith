import type { AwakeRule, ZenithSettings } from '../models/types';

/**
 * Converts reactive Svelte settings state into a clean, unproxied POJO snapshot
 * safe for structured cloning, JSON serialization, and Tauri IPC transfer.
 */
export function serializeSettingsSnapshot(settings: ZenithSettings): ZenithSettings {
  const snap = typeof (globalThis as any).$state?.snapshot === 'function'
    ? (globalThis as any).$state.snapshot(settings)
    : settings;

  return {
    launch_at_login: Boolean(snap.launch_at_login),
    clean_ai_tools: Boolean(snap.clean_ai_tools),
    clean_developer_tools: Boolean(snap.clean_developer_tools),
    clean_docker: Boolean(snap.clean_docker),
    clean_local_models: Boolean(snap.clean_local_models),
    include_rebuild_caches: Boolean(snap.include_rebuild_caches),
    theme: snap.theme ?? 'system',
    excluded_signatures: Array.isArray(snap.excluded_signatures)
      ? [...snap.excluded_signatures]
      : [],
    quick_panel_sections: Array.isArray(snap.quick_panel_sections)
      ? [...snap.quick_panel_sections]
      : ['storage', 'cleanup', 'ai_usage', 'categories', 'memory'],
    quick_panel_ai_providers: Array.isArray(snap.quick_panel_ai_providers)
      ? [...snap.quick_panel_ai_providers]
      : ['codex', 'claude', 'opencode', 'openrouter', 'antigravity'],
    dashboard_tabs: Array.isArray(snap.dashboard_tabs)
      ? [...snap.dashboard_tabs]
      : ['storage', 'docker', 'models', 'memory', 'usage', 'awake'],
    awake_rules: Array.isArray(snap.awake_rules)
      ? snap.awake_rules.map((rule: AwakeRule) => ({
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
