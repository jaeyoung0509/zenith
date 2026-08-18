import type { AwakeBehavior, AwakeRule, AwakeState } from '../models/types';
import {
  tauriDisableManualAwake,
  tauriGetAwakeState,
  tauriSetAwakeRules,
  tauriSetManualAwake,
} from '../utils/tauri';
import { settingsStore } from './settings.svelte';

class AwakeStore {
  state = $state<AwakeState>({
    is_active: false,
    active_rules_count: 0,
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
    await tauriSetAwakeRules(updated);
    await this.refresh();
  }

  async addRule(rule: AwakeRule) {
    const updated = [...settingsStore.settings.awake_rules, rule];
    await settingsStore.save({ awake_rules: updated });
    await tauriSetAwakeRules(updated);
    await this.refresh();
  }

  async setManual(durationSecs: number | null, behavior: AwakeBehavior = 'prevent_system_sleep') {
    this.isLoading = true;
    try {
      await tauriSetManualAwake(durationSecs, behavior);
      await this.refresh();
    } catch (e: any) {
      this.error = e?.toString();
    } finally {
      this.isLoading = false;
    }
  }

  async disableManual() {
    this.isLoading = true;
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
