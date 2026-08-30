import type {
  AgentActivitySnapshot,
  AgentIntegrationInfo,
  AgentIntegrationResult,
} from '../models/types';
import {
  tauriGetProjectContext,
  tauriRequestStopAgentSession,
  tauriGetAgentIntegrations,
  tauriSetupAgentIntegration,
  tauriRemoveAgentIntegration,
  tauriOpenInTerminal,
} from '../utils/tauri';

export class AgentActivityStore {
  snapshot = $state<AgentActivitySnapshot | null>(null);
  integrations = $state<AgentIntegrationInfo[]>([]);
  isLoading = $state(false);
  isIntegrationsLoading = $state(false);
  error = $state<string | null>(null);
  integrationsError = $state<string | null>(null);
  selectedProjectId = $state<string | null>(null);
  private refreshPromise: Promise<void> | null = null;
  private integrationsPromise: Promise<void> | null = null;
  private getProjectContextFn: typeof tauriGetProjectContext;
  private getAgentIntegrationsFn: typeof tauriGetAgentIntegrations;

  constructor(
    getProjectContextFn: typeof tauriGetProjectContext = tauriGetProjectContext,
    getAgentIntegrationsFn: typeof tauriGetAgentIntegrations = tauriGetAgentIntegrations,
  ) {
    this.getProjectContextFn = getProjectContextFn;
    this.getAgentIntegrationsFn = getAgentIntegrationsFn;
  }

  get activeSessionCount() {
    let count = 0;
    for (const project of this.snapshot?.projects ?? []) {
      for (const session of project.sessions) {
        if (
          session.status === 'active' ||
          session.status === 'working' ||
          session.status === 'starting'
        ) {
          count++;
        }
      }
    }
    for (const session of this.snapshot?.unassigned_sessions ?? []) {
      if (
        session.status === 'active' ||
        session.status === 'working' ||
        session.status === 'starting'
      ) {
        count++;
      }
    }
    return count;
  }

  get attentionSessionCount() {
    let count = 0;
    for (const project of this.snapshot?.projects ?? []) {
      for (const session of project.sessions) {
        if (session.attention_reason) {
          count++;
        }
      }
    }
    for (const session of this.snapshot?.unassigned_sessions ?? []) {
      if (session.attention_reason) {
        count++;
      }
    }
    return count;
  }

  get selectedProject() {
    if (!this.selectedProjectId || !this.snapshot) return null;
    return this.snapshot.projects.find((p) => p.identity.id === this.selectedProjectId) ?? null;
  }

  selectProject(id: string | null) {
    this.selectedProjectId = id;
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
      // Keep last successful observation visible
      this.error = error instanceof Error ? error.message : String(error);
    } finally {
      this.isLoading = false;
    }
  }

  async fetchIntegrations() {
    if (this.integrationsPromise) return this.integrationsPromise;
    this.integrationsPromise = this.performFetchIntegrations();
    try {
      await this.integrationsPromise;
    } finally {
      this.integrationsPromise = null;
    }
  }

  private async performFetchIntegrations() {
    this.isIntegrationsLoading = true;
    this.integrationsError = null;
    try {
      this.integrations = await this.getAgentIntegrationsFn();
    } catch (error) {
      // Keep the last successful integration state visible and expose a scoped error.
      this.integrationsError = error instanceof Error ? error.message : String(error);
    } finally {
      this.isIntegrationsLoading = false;
    }
  }

  async installIntegration(toolId: string): Promise<AgentIntegrationResult> {
    const res = await tauriSetupAgentIntegration(toolId);
    await this.fetchIntegrations();
    await this.refresh(true);
    return res;
  }

  async uninstallIntegration(toolId: string): Promise<AgentIntegrationResult> {
    const res = await tauriRemoveAgentIntegration(toolId);
    await this.fetchIntegrations();
    await this.refresh(true);
    return res;
  }

  async stopSession(sessionId: string, leaseId: string): Promise<void> {
    await tauriRequestStopAgentSession(sessionId, leaseId);
    await this.refresh(true);
  }

  async openInTerminal(path: string): Promise<void> {
    await tauriOpenInTerminal(path);
  }
}

export const agentActivityStore = new AgentActivityStore();
