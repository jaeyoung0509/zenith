import { Channel } from '@tauri-apps/api/core';
import { commands } from '../bindings/tauri';
import type {
  AiProviderUsage,
  AiUsageSnapshot,
  AiControlCenterSnapshot,
  AiControlPreferences,
  ControlCenterQuickSummary,
  AgentActivitySnapshot,
  AgentIntegrationInfo,
  AgentIntegrationResult,
  AgentQuickSummary,
  IngestedAgentEvent,
  AwakeBehavior,
  AwakeRule,
  AwakeState,
  Category,
  CleanEvent,
  CleanResult,
  DevelopmentListener,
  DiagnosticsSnapshot,
  DiskMetrics,
  DiskVolume,
  DockerStatus,
  LocalModelItem,
  MemoryMetrics,
  PlanPreview,
  PlatformCapabilities,
  RecommendationPreview,
  SafetySnapshot,
  ReleaseDevelopmentListenerResult,
  ReleaseMode,
  ScanEvent,
  ScanItem,
  ScanResult,
  SelectedApplication,
  ZenithSettings,
  ZenithSettings_Serialize,
} from '../models/types';

type Result<T, E> = { status: 'ok'; data: T } | { status: 'error'; error: E };

async function unwrap<T, E>(promise: Promise<Result<T, E>>): Promise<T> {
  const result = await promise;
  if (result.status === 'error') {
    throw new Error(String(result.error));
  }
  return result.data;
}

export const nativeApi = {
  async getProjectContext(force = false): Promise<AgentActivitySnapshot> {
    return await unwrap(commands.getProjectContext(force));
  },

  async requestStopAgentSession(sessionId: string, leaseId: string): Promise<void> {
    await unwrap(commands.requestStopAgentSession(sessionId, leaseId));
  },

  async getAgentIntegrations(): Promise<AgentIntegrationInfo[]> {
    return await unwrap(commands.getAgentIntegrations());
  },

  async setupAgentIntegration(toolId: string): Promise<AgentIntegrationResult> {
    return await unwrap(commands.setupAgentIntegration(toolId));
  },

  async removeAgentIntegration(toolId: string): Promise<AgentIntegrationResult> {
    return await unwrap(commands.removeAgentIntegration(toolId));
  },

  async getAgentQuickSummary(): Promise<AgentQuickSummary | null> {
    return await unwrap(commands.getAgentQuickSummary());
  },

  async postAgentEvent(event: IngestedAgentEvent): Promise<void> {
    await unwrap(commands.postAgentEvent(event as any));
  },

  async openInTerminal(path: string): Promise<void> {
    await unwrap(commands.openInTerminal(path));
  },

  async getAiUsage(
    force = false,
    onProvider?: (provider: AiProviderUsage) => void
  ): Promise<AiUsageSnapshot> {
    const channel = new Channel<AiProviderUsage>();
    channel.onmessage = (provider) => {
      onProvider?.(provider);
    };
    return await unwrap(commands.getAiUsage(channel, force));
  },

  async getAiControlCenter(force = false): Promise<AiControlCenterSnapshot> {
    return await unwrap(commands.getAiControlCenter(force));
  },

  async getAiControlQuickSummary(): Promise<ControlCenterQuickSummary | null> {
    return await commands.getAiControlQuickSummary();
  },

  async saveAiControlPreferences(preferences: AiControlPreferences): Promise<void> {
    await unwrap(commands.saveAiControlPreferences(preferences));
  },

  async runAiSafetyScan(): Promise<SafetySnapshot> {
    return await unwrap(commands.runAiSafetyScan());
  },

  async dismissAiSafetyFinding(findingId: string): Promise<void> {
    await unwrap(commands.dismissAiSafetyFinding(findingId));
  },

  async previewAiRecommendation(recommendationId: string): Promise<RecommendationPreview> {
    return await unwrap(commands.previewAiRecommendation(recommendationId));
  },

  async consumeAiRecommendationPreview(previewId: string): Promise<RecommendationPreview> {
    return await unwrap(commands.consumeAiRecommendationPreview(previewId));
  },

  async getAiControlGitDiff(projectId: string): Promise<string> {
    return await unwrap(commands.getAiControlGitDiff(projectId));
  },

  async connectOpenRouter(): Promise<void> {
    await unwrap(commands.connectOpenrouterOauth());
  },

  async startScan(
    onEvent: (event: ScanEvent) => void,
    categories?: Category[]
  ): Promise<ScanResult> {
    const channel = new Channel<ScanEvent>();
    channel.onmessage = (event) => {
      onEvent(event);
    };
    return await unwrap(commands.startScan(channel, categories ?? null));
  },

  async getLastScan(): Promise<ScanResult | null> {
    return await commands.getLastScan();
  },

  async createPlan(scanId: string, items: ScanItem[]): Promise<PlanPreview> {
    return await unwrap(
      commands.createDeletePlan(
        scanId,
        items.map((item) => item.id)
      )
    );
  },

  async executeClean(
    plan: PlanPreview,
    onEvent: (event: CleanEvent) => void
  ): Promise<CleanResult> {
    const channel = new Channel<CleanEvent>();
    channel.onmessage = (event) => {
      onEvent(event);
    };
    return await unwrap(commands.executeClean(plan.id, channel));
  },

  async getMemoryMetrics(): Promise<MemoryMetrics> {
    return await unwrap(commands.getMemoryMetrics());
  },

  async terminateProcessGroup(name: string, force: boolean): Promise<number> {
    return await unwrap(commands.terminateProcessGroup(name, force));
  },

  async pickKeepAwakeApplication(): Promise<SelectedApplication | null> {
    return await unwrap(commands.pickKeepAwakeApplication());
  },

  async getDiskMetrics(): Promise<DiskMetrics> {
    return await unwrap(commands.getDiskMetrics());
  },

  async getDiskVolumes(): Promise<DiskVolume[]> {
    return await unwrap(commands.getDiskVolumes());
  },

  async openDiskUtility(): Promise<void> {
    await unwrap(commands.openDiskUtility());
  },

  async getDockerStatus(): Promise<DockerStatus> {
    return await unwrap(commands.getDockerStatus());
  },

  async pruneDocker(signatureId: string): Promise<number> {
    return await unwrap(commands.pruneDockerTarget(signatureId));
  },

  async getLocalModels(): Promise<LocalModelItem[]> {
    return await unwrap(commands.getLocalModels());
  },

  async deleteLocalModel(modelId: string): Promise<number> {
    return await unwrap(commands.deleteLocalModel(modelId));
  },

  async getAwakeState(): Promise<AwakeState> {
    return await unwrap(commands.getAwakeState());
  },

  async setAwakeRules(rules: AwakeRule[]): Promise<void> {
    await unwrap(commands.setAwakeRules(rules));
  },

  async setManualAwake(
    durationSecs: number | null,
    behavior: AwakeBehavior
  ): Promise<void> {
    await unwrap(commands.setManualAwake(durationSecs, behavior));
  },

  async disableManualAwake(): Promise<void> {
    await unwrap(commands.disableManualAwake());
  },

  async getSettings(): Promise<ZenithSettings_Serialize> {
    return await unwrap(commands.getSettings());
  },

  async saveSettings(settings: ZenithSettings): Promise<void> {
    await unwrap(commands.saveSettings(settings));
  },

  async revealInFinder(path: string): Promise<void> {
    await unwrap(commands.revealInFinder(path));
  },

  async openDashboard(): Promise<void> {
    await unwrap(commands.openDashboardWindow());
  },

  async toggleQuick(): Promise<void> {
    await unwrap(commands.toggleQuickPanel());
  },

  async getAppVersion(): Promise<string> {
    try {
      return await commands.getAppVersion();
    } catch {
      return typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : '0.1.0';
    }
  },

  async getPlatformCapabilities(): Promise<PlatformCapabilities> {
    return await commands.getPlatformCapabilities();
  },

  async getDiagnostics(): Promise<DiagnosticsSnapshot> {
    return await unwrap(commands.getDiagnostics());
  },

  async openLogsFolder(): Promise<void> {
    await unwrap(commands.openLogsFolder());
  },

  async listDevelopmentListeners(): Promise<DevelopmentListener[]> {
    return await unwrap(commands.listDevelopmentListeners());
  },

  async releaseDevelopmentListener(
    id: string,
    mode: ReleaseMode
  ): Promise<ReleaseDevelopmentListenerResult> {
    return await unwrap(commands.releaseDevelopmentListener(id, mode));
  },

  async hideCurrentWindow(): Promise<void> {
    const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
    await getCurrentWebviewWindow().hide();
  },
};
