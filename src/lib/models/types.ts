// Re-export generated types from Tauri Specta bindings
export type {
  AiProviderUsage,
  AiControlPreferences,
  AiUsageSnapshot,
  AgentActivitySnapshot,
  AgentActivityStatus,
  AgentAdapterHealth,
  AgentAdapterState,
  AgentEvidence,
  AgentIntegrationInfo,
  AgentIntegrationResult,
  AgentLifecycleEvent,
  AgentNotificationPreferences,
  AgentQuickSessionRow,
  AgentQuickSummary,
  AgentSession,
  AttentionReason,
  IngestedAgentEvent,
  AppInstallSource,
  AppRelatedConfidence,
  AppRelatedItem,
  AppRelatedKind,
  AppUninstallInspection,
  AwakeBehavior,
  AwakeRule,
  AwakeRuleEvaluation,
  AwakeRuleStatus,
  AwakeState,
  AuditEntry,
  AutopilotPreferences,
  BudgetPeriod,
  BudgetStatus,
  Category,
  CategoryResult,
  CleanEvent,
  CleanFailureReason,
  CleanItemResult,
  CleanResult,
  CleanStatus,
  ControlCenterQuickSummary,
  DashboardTab_Deserialize,
  DashboardTab_Serialize,
  DevelopmentListener,
  DeveloperArtifact,
  DeveloperArtifactKind,
  DeveloperArtifactScanEvent,
  DeveloperArtifactScanResult,
  DeveloperArtifactStatus,
  DeveloperEcosystem,
  DeveloperWorkspace,
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
  InstalledApp,
  LargeFileFilter,
  LargeFileItem,
  LargeFileKind,
  LargeFileScanEvent,
  LargeFileScanRequest,
  LargeFileScanResult,
  ListenerExposure,
  ListenerProtocol,
  LocalModelItem,
  LocalAlertBudget,
  ManualProviderUsage,
  MemoryMetrics,
  MemoryPressure,
  MoneyMicros,
  NormalizedSafetyEvidence,
  ObservationPeriod,
  ObservationQuality,
  ObservationScope,
  ObservationSourceKind,
  ModelSource,
  PlanPreview,
  PlanTargetPreview,
  PowerCondition,
  PowerSourceType,
  ProcessMemory,
  ProjectContext,
  ProjectIdentity,
  ProviderMetric,
  ProviderObservation,
  QuickPanelSection,
  RecommendationKind,
  ReleaseDevelopmentListenerResult,
  ReleaseMode,
  ReleaseOutcome,
  RiskSummary,
  RiskTier,
  ResourceAttribution,
  SafetyFinding,
  SafetyFindingKind,
  SafetySnapshot,
  FindingSeverity,
  GitChangeSummary,
  ScanEvent,
  ScanItem,
  ScanResult,
  SnapshotQuality,
  SelectedApplication,
  TrashItemResult,
  TrashPlanPreview,
  TrashResult,
  UsageSummary,
  UsageSupport,
  UsageWindow,
  ZenithSettings_Deserialize,
  ZenithSettings_Serialize,
} from '../bindings/tauri';

import type {
  AiControlCenterSnapshot_Serialize,
  DashboardRoute_Serialize,
  DashboardTab_Serialize,
  Recommendation_Serialize,
  RecommendationPreview_Serialize,
  ZenithSettings_Serialize,
} from '../bindings/tauri';

// In the frontend runtime, settings and tabs are always fully resolved/serialized shapes
export type ZenithSettings = ZenithSettings_Serialize;
export type DashboardTab = DashboardTab_Serialize;
export type DashboardRoute = DashboardRoute_Serialize;
export type Recommendation = Recommendation_Serialize;
export type RecommendationPreview = RecommendationPreview_Serialize;
export type AiControlCenterSnapshot = AiControlCenterSnapshot_Serialize;

// Frontend-specific helper types and aliases
export type AiProviderId =
  | 'codex'
  | 'claude'
  | 'opencode'
  | 'openrouter'
  | 'antigravity'
  | 'cursor'
  | 'grok';

export type CleanStrategy =
  | 'delete_contents'
  | 'delete_directory'
  | 'external_command'
  | 'docker_prune'
  | 'manual';
