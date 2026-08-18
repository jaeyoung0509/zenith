export type Category = 'ai' | 'developer' | 'container' | 'model' | 'system';

export type RiskTier = 'safe' | 'rebuild' | 'manual';

export type CleanStrategy =
  | 'delete_contents'
  | 'delete_directory'
  | 'external_command'
  | 'docker_prune'
  | 'manual';

export interface FileSize {
  logical: number;
  allocated?: number;
}

export interface ScanItem {
  id: string;
  signature_id: string;
  name: string;
  category: Category;
  risk: RiskTier;
  path: string;
  size: FileSize;
  file_count: number;
  description: string;
  is_selected: boolean;
  last_modified?: number;
  exists: boolean;
}

export interface CategoryResult {
  category: Category;
  display_name: string;
  items: ScanItem[];
  total_bytes: number;
  safe_bytes: number;
  rebuild_bytes: number;
  manual_bytes: number;
}

export interface ScanResult {
  scan_id: string;
  started_at: number;
  finished_at: number;
  categories: CategoryResult[];
  total_bytes: number;
  safe_bytes: number;
  rebuild_bytes: number;
  manual_bytes: number;
}

export type ScanEvent =
  | { type: 'Started'; scan_id: string }
  | { type: 'CategoryStarted'; category: Category }
  | { type: 'ItemFound'; item: ScanItem }
  | { type: 'CategoryFinished'; category: Category; bytes: number; item_count: number }
  | { type: 'Finished'; result: ScanResult }
  | { type: 'Error'; message: string };

export interface RiskSummary {
  safe_count: number;
  rebuild_count: number;
  manual_count: number;
  safe_bytes: number;
  rebuild_bytes: number;
  manual_bytes: number;
}

export interface DeleteTarget {
  item_id: string;
  signature_id: string;
  name: string;
  path: string;
  strategy: CleanStrategy;
  expected_bytes: number;
  risk: RiskTier;
}

export interface DeletePlan {
  id: string;
  targets: DeleteTarget[];
  expected_reclaim_bytes: number;
  risk: RiskSummary;
  created_at: number;
}

export type CleanFailureReason =
  | 'permission_denied'
  | 'changed_since_scan'
  | 'not_found'
  | 'in_use'
  | 'blacklisted'
  | 'external_command_failed'
  | 'unknown';

export interface CleanItemResult {
  item_id: string;
  name: string;
  path: string;
  success: boolean;
  bytes_reclaimed: number;
  failure_reason?: CleanFailureReason;
  error_message?: string;
}

export interface CleanResult {
  plan_id: string;
  started_at: number;
  finished_at: number;
  total_reclaimed_bytes: number;
  total_failed_bytes: number;
  items: CleanItemResult[];
  actual_disk_free_delta?: number;
}

export type CleanEvent =
  | { type: 'Started'; plan_id: string; total_targets: number; expected_bytes: number }
  | { type: 'ItemStarted'; item_id: string; name: string; index: number; total: number }
  | { type: 'ItemFinished'; item_id: string; name: string; success: boolean; reclaimed_bytes: number; error?: string }
  | { type: 'Finished'; result: CleanResult }
  | { type: 'Error'; message: string };

export type MemoryPressure = 'normal' | 'warning' | 'critical';

export interface ProcessMemory {
  pid: number;
  name: string;
  memory_bytes: number;
  process_count: number;
  can_terminate: boolean;
}

export interface MemoryMetrics {
  total_bytes: number;
  used_bytes: number;
  available_bytes: number;
  free_bytes: number;
  compressed_bytes: number;
  swap_used_bytes: number;
  swap_total_bytes: number;
  pressure: MemoryPressure;
  top_processes: ProcessMemory[];
  timestamp: number;
}

export interface DiskMetrics {
  mount_point: string;
  total_bytes: number;
  used_bytes: number;
  free_bytes: number;
  available_bytes: number;
  percent_used: number;
}

export interface DiskVolume {
  name: string;
  mount_point: string;
  file_system: string;
  disk_type: string;
  total_bytes: number;
  used_bytes: number;
  available_bytes: number;
  percent_used: number;
  is_removable: boolean;
  is_primary: boolean;
}

export interface DockerImageItem {
  id: string;
  repository: string;
  tag: string;
  size_bytes: number;
  is_dangling: boolean;
  is_in_use: boolean;
}

export interface DockerContainerItem {
  id: string;
  name: string;
  image: string;
  state: string;
  size_bytes: number;
  is_running: boolean;
}

export interface DockerVolumeItem {
  name: string;
  driver: string;
  size_bytes: number;
  is_in_use: boolean;
}

export interface DockerOverview {
  images_bytes: number;
  dangling_images_bytes: number;
  build_cache_bytes: number;
  stopped_containers_bytes: number;
  volumes_bytes: number;
  total_bytes: number;
  safe_cleanable_bytes: number;
}

export interface DockerStatus {
  is_available: boolean;
  is_running: boolean;
  version?: string;
  error_message?: string;
  overview?: DockerOverview;
  images: DockerImageItem[];
  containers: DockerContainerItem[];
  volumes: DockerVolumeItem[];
}

export type ModelSource = 'ollama' | 'huggingface' | 'lmstudio' | 'mlx';

export interface LocalModelItem {
  id: string;
  name: string;
  source: ModelSource;
  path: string;
  size_bytes: number;
  format?: string;
  parameter_size?: string;
  quantization?: string;
  last_modified?: number;
}

export type AwakeBehavior = 'prevent_system_sleep' | 'keep_display_awake';

export interface AwakeRule {
  id: string;
  app_name: string;
  executable_pattern: string;
  behavior: AwakeBehavior;
  enabled: boolean;
}

export interface SelectedApplication {
  name: string;
  executable_pattern: string;
  path: string;
}

export interface AwakeState {
  is_active: boolean;
  behavior?: AwakeBehavior;
  trigger_source?: string;
  active_process_name?: string;
  manual_expires_at?: number;
  active_rules_count: number;
}

export interface ZenithSettings {
  launch_at_login: boolean;
  clean_ai_tools: boolean;
  clean_developer_tools: boolean;
  clean_docker: boolean;
  clean_local_models: boolean;
  include_rebuild_caches: boolean;
  theme: string;
  excluded_signatures: string[];
  awake_rules: AwakeRule[];
  quick_panel_sections: QuickPanelSection[];
  quick_panel_ai_providers: AiProviderId[];
}

export type QuickPanelSection = 'storage' | 'cleanup' | 'ai_usage' | 'categories' | 'memory';
export type AiProviderId = 'codex' | 'claude' | 'opencode' | 'openrouter' | 'antigravity';

export type UsageSupport = 'live' | 'local' | 'manual';

export interface UsageWindow {
  label: string;
  used_percent: number;
  resets_at?: number;
}

export interface UsageSummary {
  lifetime_tokens?: number;
  last_7d_tokens?: number;
  peak_daily_tokens?: number;
  current_streak_days?: number;
  local_sessions?: number;
  local_cost_usd?: number;
  usage_usd?: number;
  limit_remaining_usd?: number;
}

export interface AiProviderUsage {
  id: string;
  name: string;
  installed: boolean;
  connected: boolean;
  auth_label: string;
  status_message: string;
  support: UsageSupport;
  windows: UsageWindow[];
  summary: UsageSummary;
  action_url?: string;
}

export interface AiUsageSnapshot {
  providers: AiProviderUsage[];
  fetched_at: number;
}
