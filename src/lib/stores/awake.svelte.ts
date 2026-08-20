import type { AwakeBehavior, AwakeRule, AwakeState } from '../models/types';
import {
  tauriDisableManualAwake,
  tauriGetAwakeState,
  tauriSetManualAwake,
} from '../utils/tauri';
import { settingsStore } from './settings.svelte';

class AwakeStore {
  state = $state<AwakeState>({
    is_active: false,
    behavior: null,
    trigger_source: null,
    active_process_name: null,
    active_rule_id: null,
    manual_expires_at: null,
    active_rules_count: 0,
    power_source: 'unknown',
    last_error: null,
    rule_evaluations: [],
  });
  isLoading = $state(false);
  error = $state<string | null>(null);

  async refresh() {
    try {
      this.state = await tauriGetAwakeState();
    } catch (e: any) {
      this.error = e?.toString();
    }
  }

  async toggleRule(ruleId: string) {
    const current = settingsStore.settings.awake_rules;
    const updated = current.map((r) =>
      r.id === ruleId ? { ...r, enabled: !r.enabled } : r
    );
    await settingsStore.save({ awake_rules: updated });
    await this.refresh();
  }

  async addRule(rule: AwakeRule) {
    const updated = [...settingsStore.settings.awake_rules, rule];
    await settingsStore.save({ awake_rules: updated });
    await this.refresh();
  }

  async deleteRule(ruleId: string) {
    const current = settingsStore.settings.awake_rules;
    const updated = current.filter((r) => r.id !== ruleId);
    await settingsStore.save({ awake_rules: updated });
    await this.refresh();
  }

  async updateRule(rule: AwakeRule) {
    const current = settingsStore.settings.awake_rules;
    const updated = current.map((r) => (r.id === rule.id ? rule : r));
    await settingsStore.save({ awake_rules: updated });
    await this.refresh();
  }

  async setManual(durationSecs: number | null, behavior: AwakeBehavior = 'prevent_system_sleep') {
    this.isLoading = true;
    this.error = null;
    try {
      await tauriSetManualAwake(durationSecs, behavior);
      await this.refresh();
    } catch (e: any) {
      this.error = e?.toString();
      throw e;
    } finally {
      this.isLoading = false;
    }
  }

  async disableManual() {
    this.isLoading = true;
    this.error = null;
    try {
      await tauriDisableManualAwake();
      await this.refresh();
    } catch (e: any) {
      this.error = e?.toString();
    } finally {
      this.isLoading = false;
    }
  }
}

export const awakeStore = new AwakeStore();
