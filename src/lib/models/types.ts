// Re-export generated types from Tauri Specta bindings
export type {
  AiProviderUsage,
  AiUsageSnapshot,
  AwakeBehavior,
  AwakeRule,
  AwakeRuleEvaluation,
  AwakeRuleStatus,
  AwakeState,
  Category,
  CategoryResult,
  CleanEvent,
  CleanFailureReason,
  CleanItemResult,
  CleanResult,
  CleanStatus,
  DashboardTab_Deserialize,
  DashboardTab_Serialize,
  DiagnosticsSnapshot,
  DiskMetrics,
  DiskVolume,
  DockerContainerItem,
  DockerImageItem,
  DockerOverview,
  DockerResourceUsage,
  DockerStatus,
  DockerVolumeItem,
  FileSize,
  LocalModelItem,
  MemoryMetrics,
  MemoryPressure,
  ModelSource,
  PlanPreview,
  PlanTargetPreview,
  PowerCondition,
  PowerSourceType,
  ProcessMemory,
  QuickPanelSection,
  RiskSummary,
  RiskTier,
  ScanEvent,
  ScanItem,
  ScanResult,
  SelectedApplication,
  UsageSummary,
  UsageSupport,
  UsageWindow,
  ZenithSettings_Deserialize,
  ZenithSettings_Serialize,
} from '../bindings/tauri';

import type {
  DashboardTab_Serialize,
  ZenithSettings_Serialize,
} from '../bindings/tauri';

// In the frontend runtime, settings and tabs are always fully resolved/serialized shapes
export type ZenithSettings = ZenithSettings_Serialize;
export type DashboardTab = DashboardTab_Serialize;

// Frontend-specific helper types and aliases
export type AiProviderId = 'codex' | 'claude' | 'opencode' | 'openrouter' | 'antigravity';

export type CleanStrategy =
  | 'delete_contents'
  | 'delete_directory'
  | 'external_command'
  | 'docker_prune'
  | 'manual';
