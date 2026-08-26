import type { AgentActivitySnapshot } from '../models/types';
import { tauriGetProjectContext } from '../utils/tauri';

export class AgentActivityStore {
  snapshot = $state<AgentActivitySnapshot | null>(null);
  isLoading = $state(false);
  error = $state<string | null>(null);
  private refreshPromise: Promise<void> | null = null;
  private getProjectContextFn: typeof tauriGetProjectContext;

  constructor(getProjectContextFn: typeof tauriGetProjectContext = tauriGetProjectContext) {
    this.getProjectContextFn = getProjectContextFn;
  }

  get activeSessionCount() {
    const assigned = this.snapshot?.projects.reduce((sum, project) => sum + project.sessions.length, 0) ?? 0;
    return assigned + (this.snapshot?.unassigned_sessions.length ?? 0);
  }

  async refresh(force = false) {
    if (this.refreshPromise) return this.refreshPromise;
    this.refreshPromise = this.performRefresh(force);
    try {
      await this.refreshPromise;
    } finally {
      this.refreshPromise = null;
    }
  }

  private async performRefresh(force: boolean) {
    this.isLoading = true;
    try {
      this.snapshot = await this.getProjectContextFn(force);
      this.error = null;
    } catch (error) {
      // Keep the last successful observation visible; a refresh failure is not
      // allowed to erase useful local state.
      this.error = error instanceof Error ? error.message : String(error);
    } finally {
      this.isLoading = false;
    }
  }
}

export const agentActivityStore = new AgentActivityStore();
