import { api, isTauri as isTauriCheck } from '../api';
import type {
  AiUsageSnapshot,
  AwakeBehavior,
  AwakeRule,
  AwakeState,
  Category,
  CleanEvent,
  CleanResult,
  DiskMetrics,
  DiskVolume,
  DockerStatus,
  LocalModelItem,
  MemoryMetrics,
  PlanPreview,
  ScanEvent,
  ScanItem,
  ScanResult,
  SelectedApplication,
  ZenithSettings,
} from '../models/types';

export const isTauri = isTauriCheck();

export function tauriGetAiUsage(force = false): Promise<AiUsageSnapshot> {
  return api.getAiUsage(force);
}

export function tauriConnectOpenRouter(): Promise<void> {
  return api.connectOpenRouter();
}

export function tauriScan(
  onEvent: (event: ScanEvent) => void,
  categories?: Category[]
): Promise<ScanResult> {
  return api.startScan(onEvent, categories);
}

export function tauriGetLastScan(): Promise<ScanResult | null> {
  return api.getLastScan();
}

export function tauriCreatePlan(scanId: string, items: ScanItem[]): Promise<PlanPreview> {
  return api.createPlan(scanId, items);
}

export function tauriExecuteClean(
  plan: PlanPreview,
  onEvent: (event: CleanEvent) => void
): Promise<CleanResult> {
  return api.executeClean(plan, onEvent);
}

export function tauriGetMemoryMetrics(): Promise<MemoryMetrics> {
  return api.getMemoryMetrics();
}

export function tauriTerminateProcessGroup(name: string, force: boolean): Promise<number> {
  return api.terminateProcessGroup(name, force);
}

export function tauriPickKeepAwakeApplication(): Promise<SelectedApplication | null> {
  return api.pickKeepAwakeApplication();
}

export function tauriGetDiskMetrics(): Promise<DiskMetrics> {
  return api.getDiskMetrics();
}

export function tauriGetDiskVolumes(): Promise<DiskVolume[]> {
  return api.getDiskVolumes();
}

export function tauriOpenDiskUtility(): Promise<void> {
  return api.openDiskUtility();
}

export function tauriGetDockerStatus(): Promise<DockerStatus> {
  return api.getDockerStatus();
}

export function tauriPruneDocker(signatureId: string): Promise<number> {
  return api.pruneDocker(signatureId);
}

export function tauriGetLocalModels(): Promise<LocalModelItem[]> {
  return api.getLocalModels();
}

export function tauriDeleteLocalModel(modelId: string): Promise<number> {
  return api.deleteLocalModel(modelId);
}

export function tauriGetAwakeState(): Promise<AwakeState> {
  return api.getAwakeState();
}

export function tauriSetAwakeRules(rules: AwakeRule[]): Promise<void> {
  return api.setAwakeRules(rules);
}

export function tauriSetManualAwake(
  durationSecs: number | null,
  behavior: AwakeBehavior
): Promise<void> {
  return api.setManualAwake(durationSecs, behavior);
}

export function tauriDisableManualAwake(): Promise<void> {
  return api.disableManualAwake();
}

export function tauriGetSettings(): Promise<ZenithSettings> {
  return api.getSettings();
}

export function tauriSaveSettings(settings: ZenithSettings): Promise<void> {
  return api.saveSettings(settings);
}

export function tauriRevealInFinder(path: string): Promise<void> {
  return api.revealInFinder(path);
}

export function tauriOpenDashboard(): Promise<void> {
  return api.openDashboard();
}

export function tauriToggleQuick(): Promise<void> {
  return api.toggleQuick();
}

export function tauriGetAppVersion(): Promise<string> {
  return api.getAppVersion();
}

export function tauriHideCurrentWindow(): Promise<void> {
  return api.hideCurrentWindow();
}
