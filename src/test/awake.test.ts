import { describe, expect, it } from 'vitest';
import type { AwakeRule, AwakeState } from '../lib/models/types';

describe('awake models and state handling', () => {
  it('correctly models power conditions and evaluations', () => {
    const state: AwakeState = {
      is_active: true,
      behavior: 'prevent_system_sleep',
      trigger_source: 'Triggered by Codex',
      active_process_name: 'Codex',
      active_rule_id: 'rule.codex',
      active_rules_count: 2,
      power_source: 'ac',
      rule_evaluations: [
        {
          rule_id: 'rule.codex',
          status: 'active',
          is_process_running: true,
          is_power_eligible: true,
        },
        {
          rule_id: 'rule.claude',
          status: 'waiting_process',
          is_process_running: false,
          is_power_eligible: true,
        },
      ],
    };

    expect(state.is_active).toBe(true);
    expect(state.power_source).toBe('ac');
    expect(state.rule_evaluations.length).toBe(2);
    expect(state.rule_evaluations[0].status).toBe('active');
  });

  it('supports AC power only and always rules', () => {
    const acRule: AwakeRule = {
      id: 'rule.codex',
      app_name: 'Codex',
      executable_pattern: 'codex',
      behavior: 'prevent_system_sleep',
      power_condition: 'ac_power_only',
      enabled: true,
    };

    const alwaysRule: AwakeRule = {
      id: 'rule.render',
      app_name: 'Render Farm',
      executable_pattern: 'blender',
      behavior: 'keep_display_awake',
      power_condition: 'always',
      enabled: true,
    };

    expect(acRule.power_condition).toBe('ac_power_only');
    expect(alwaysRule.power_condition).toBe('always');
  });
});
