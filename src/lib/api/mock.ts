import type {
  AgentActivitySnapshot,
  AiControlCenterSnapshot,
  AiControlPreferences,
  AiUsageSnapshot,
  AwakeBehavior,
  AwakeRule,
  AwakeState,
  Category,
  CleanEvent,
  CleanItemResult,
  CleanResult,
  ControlCenterQuickSummary,
  AgentIntegrationInfo,
  AgentIntegrationResult,
  AgentQuickSummary,
  IngestedAgentEvent,
  DevelopmentListener,
  DiagnosticsSnapshot,
  DiskMetrics,
  DiskVolume,
  DockerStatus,
  LocalModelItem,
  MemoryMetrics,
  PlanPreview,
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
import type { nativeApi } from './native';

type ZenithApi = typeof nativeApi;

let mockControlPreferences: AiControlPreferences = {
  budgets: [],
  manual_usage: [],
  autopilot: {
    keep_awake_for_verified_sessions: false,
    keep_awake_ac_only: true,
    notify_on_battery: false,
    notify_on_memory_pressure: false,
    notify_on_session_completion: false,
    recommendation_cooldown_seconds: 900,
  },
  dismissed_findings: [],
  audit_retention_days: 30,
};

function mockControlSnapshot(): AiControlCenterSnapshot {
  const now = Math.floor(Date.now() / 1000);
  return {
    observed_at: now,
    providers: [
      {
        provider_id: 'codex-subscription', display_name: 'Codex subscription',
        source_kind: 'live_quota', source_id: 'codex-app-server', scope: 'subscription',
        observed_at: now, period: { starts_at: null, ends_at: now + 172800, resets_at: now + 172800, label: 'Weekly' },
        fresh_for_seconds: 300, quality: 'fresh', installed: true, connected: true,
        status_message: 'Live subscription window from Codex.',
        metrics: [{ label: 'Weekly usage', tokens: null, cost: null, used_basis_points: 7000 }],
        action_url: null, partial_error: null,
      },
      {
        provider_id: 'opencode-local', display_name: 'OpenCode local activity',
        source_kind: 'local_estimate', source_id: 'opencode-stats', scope: 'local_sessions',
        observed_at: now, period: { starts_at: now - 604800, ends_at: now, resets_at: null, label: 'Last 7 days' },
        fresh_for_seconds: 300, quality: 'fresh', installed: true, connected: true,
        status_message: 'Local estimate; not a provider bill.',
        metrics: [{ label: 'Local estimate', tokens: 1280000, cost: { micros: 1420000, currency: 'USD' }, used_basis_points: null }],
        action_url: null, partial_error: null,
      },
      ...['openai-api', 'openrouter', 'anthropic-api', 'claude-individual', 'cursor-individual', 'antigravity', 'gemini-enterprise', 'xai-api', 'grok-individual'].map((provider_id) => ({
        provider_id, display_name: provider_id.replaceAll('-', ' '), source_kind: 'manual' as const,
        source_id: 'manual-entry', scope: provider_id.includes('individual') || provider_id === 'antigravity' ? 'subscription' as const : 'api_key' as const,
        observed_at: now, period: { starts_at: null, ends_at: null, resets_at: null, label: 'Not reported' },
        fresh_for_seconds: 0, quality: 'unavailable' as const, installed: false, connected: false,
        status_message: 'No authoritative usage source is connected.', metrics: [], action_url: null, partial_error: null,
      })),
    ],
    budget_statuses: [],
    resources: [{ session_id: 'session-codex-preview', project_id: 'project-zenith-preview', tool_name: 'Codex CLI', cpu_percent: 6.4, memory_bytes: 490733568, process_count: 1, duration_seconds: 1320, open_dev_ports: 1, power_eligible: true, confidence: 'verified', reason: 'Canonical session and project identity matched.', mutable_actions_allowed: true }],
    recommendations: [{ id: 'recommendation-port-preview', kind: 'development_port', title: 'Review open development port', message: 'A verified project session has an open development listener.', created_at: now, cooldown_until: now + 900, session_id: 'session-codex-preview', project_id: 'project-zenith-preview', action_label: 'Preview', destination: 'development_servers' }],
    safety: { observed_at: now, quality: 'unavailable', findings: [], scanned_files: 0, skipped_files: 0, status_message: 'Run an explicit bounded inspection.' },
    git_summaries: [{ project_id: 'project-zenith-preview', baseline_head: 'abc1234', current_head: 'abc1234', baseline_at: now - 1200, added: 0, modified: 2, deleted: 0, renamed: 0, untracked: 1, changed_paths: ['src/routes/dashboard/AiControlCenterView.svelte', 'src-tauri/src/ai_control_center/mod.rs'], available: true, status_message: '3 paths changed after the Zenith baseline.' }],
    audit: [],
    quick_summary: { observed_at: now, active_sessions: 1, budget_alerts: 0, safety_findings: 0, quality: 'fresh' },
    keep_awake_active: false,
    partial_errors: [],
  };
}

export const mockApi = {
  async getProjectContext(_force = false): Promise<AgentActivitySnapshot> {
    const observedAt = Math.floor(Date.now() / 1000);
    return {
      observed_at: observedAt,
      quality: 'fresh',
      projects: [
        {
          identity: {
            id: 'project-zenith-preview',
            display_name: 'zenith',
            location_hint: 'Myproject/clean1',
            display_path: '~/Myproject/clean1',
            repository_id: 'repository-zenith-preview',
            worktree_id: null,
            is_worktree: false,
            branch: 'feature/75-agent-project-cockpit',
            is_dirty: true,
            is_detached: false,
          },
          sessions: [
            {
              id: 'session-antigravity-preview',
              tool_id: 'antigravity',
              tool_name: 'Antigravity',
              status: 'working',
              attention_reason: null,
              evidence: 'vendor_event',
              observed_at: observedAt,
              started_at: observedAt - 1320,
              elapsed_seconds: 1320,
              cpu_percent: 6.4,
              memory_bytes: 468 * 1024 * 1024,
              project_id: 'project-zenith-preview',
              worktree_id: null,
              detail: 'Vendor event confirmed',
              can_stop: true,
              stop_lease_id: 'lease-antigravity-mock',
            },
          ],
          last_seen_at: observedAt,
          dev_ports: [5173],
          artifact_size_bytes: 1024 * 1024 * 50,
        },
        {
          identity: {
            id: 'project-design-preview',
            display_name: 'design-system',
            location_hint: 'worktrees/design-system',
            display_path: '~/worktrees/design-system',
            repository_id: 'repository-design-preview',
            worktree_id: 'worktree-design-preview',
            is_worktree: true,
            branch: 'feature/token-audit',
            is_dirty: false,
            is_detached: false,
          },
          sessions: [
            {
              id: 'session-claude-preview',
              tool_id: 'claude',
              tool_name: 'Claude Code',
              status: 'waiting_for_user',
              attention_reason: 'approval',
              evidence: 'process_observed',
              observed_at: observedAt,
              started_at: observedAt - 420,
              elapsed_seconds: 420,
              cpu_percent: 2.1,
              memory_bytes: 224 * 1024 * 1024,
              project_id: 'project-design-preview',
              worktree_id: 'worktree-design-preview',
              detail: 'Waiting for tool approval',
              can_stop: true,
              stop_lease_id: 'lease-claude-mock',
            },
          ],
          last_seen_at: observedAt,
          dev_ports: [3000],
          artifact_size_bytes: 1024 * 1024 * 120,
        },
      ],
      unassigned_sessions: [],
      adapters: [
        { tool_id: 'antigravity', display_name: 'Antigravity', state: 'connected', evidence: 'vendor_event', message: 'Local status hook connected and delivering lifecycle events.', installed_version: '2.0.0' },
        { tool_id: 'claude', display_name: 'Claude Code', state: 'integration_available', evidence: 'process_observed', message: 'Process observed · local integration available to install.', installed_version: '1.0.0' },
        { tool_id: 'cursor', display_name: 'Cursor Agent CLI', state: 'integration_available', evidence: null, message: 'Not observed · local integration available.', installed_version: null },
        { tool_id: 'grok', display_name: 'Grok Build', state: 'integration_available', evidence: null, message: 'Not observed · local integration available.', installed_version: null },
        { tool_id: 'copilot', display_name: 'GitHub Copilot CLI', state: 'integration_available', evidence: null, message: 'Not observed · local integration available.', installed_version: null },
        { tool_id: 'gemini', display_name: 'Gemini CLI (legacy / enterprise)', state: 'process_only', evidence: null, message: 'Process-only observation.', installed_version: null },
        { tool_id: 'codex', display_name: 'Codex CLI', state: 'process_only', evidence: null, message: 'Process-only observation.', installed_version: null },
        { tool_id: 'opencode', display_name: 'OpenCode', state: 'process_only', evidence: null, message: 'Process-only baseline.', installed_version: null },
      ],
      partial_errors: [],
    };
  },

  async requestStopAgentSession(_sessionId: string, _leaseId: string): Promise<void> {
    // Mock successful stop
  },

  async getAgentIntegrations(): Promise<AgentIntegrationInfo[]> {
    return [
      { tool_id: 'antigravity', display_name: 'Antigravity', supported: true, installed: true, integration_active: true, config_path: '~/.gemini/antigravity/hooks.json', description: 'Google primary agent CLI.' },
      { tool_id: 'claude', display_name: 'Claude Code', supported: true, installed: true, integration_active: false, config_path: '~/.claude/settings.json', description: 'Claude Code official lifecycle and notification hooks.' },
      { tool_id: 'cursor', display_name: 'Cursor Agent CLI', supported: true, installed: false, integration_active: false, config_path: '~/.cursor/hooks.json', description: 'Cursor local lifecycle hooks.' },
      { tool_id: 'grok', display_name: 'Grok Build', supported: true, installed: false, integration_active: false, config_path: '~/.grok/hooks.json', description: 'xAI Grok Build lifecycle hooks.' },
      { tool_id: 'copilot', display_name: 'GitHub Copilot CLI', supported: true, installed: false, integration_active: false, config_path: '~/.copilot/hooks.json', description: 'GitHub Copilot CLI lifecycle hooks.' },
      { tool_id: 'gemini', display_name: 'Gemini CLI (legacy / enterprise)', supported: false, installed: false, integration_active: false, config_path: null, description: 'Process-only observation.' },
      { tool_id: 'codex', display_name: 'Codex CLI', supported: false, installed: false, integration_active: false, config_path: null, description: 'Process-only observation.' },
      { tool_id: 'opencode', display_name: 'OpenCode', supported: false, installed: false, integration_active: false, config_path: null, description: 'Process-only observation.' },
    ];
  },

  async setupAgentIntegration(toolId: string): Promise<AgentIntegrationResult> {
    return { tool_id: toolId, success: true, message: `Integration for ${toolId} installed.` };
  },

  async removeAgentIntegration(toolId: string): Promise<AgentIntegrationResult> {
    return { tool_id: toolId, success: true, message: `Integration for ${toolId} removed.` };
  },

  async getAgentQuickSummary(): Promise<AgentQuickSummary | null> {
    return {
      active_count: 2,
      attention_count: 1,
      sessions: [
        {
          session_id: 'session-antigravity-preview',
          tool_name: 'Antigravity',
          project_name: 'zenith',
          status: 'working',
          evidence: 'vendor_event',
          elapsed_seconds: 1320,
        },
        {
          session_id: 'session-claude-preview',
          tool_name: 'Claude Code',
          project_name: 'design-system',
          status: 'waiting_for_user',
          evidence: 'process_observed',
          elapsed_seconds: 420,
        },
      ],
    };
  },

  async postAgentEvent(_event: IngestedAgentEvent): Promise<void> {
    // Mock event receipt
  },

  async openInTerminal(_path: string): Promise<void> {
    // Mock open in terminal
  },

  async getAiUsage(_force = false): Promise<AiUsageSnapshot> {
    return {
      fetched_at: Math.floor(Date.now() / 1000),
      providers: [
        {
          id: 'codex',
          name: 'Codex',
          installed: true,
          connected: true,
          auth_label: 'plus · OAuth',
          status_message: 'Live account limits from the official Codex app-server.',
          support: 'live',
          windows: [{ label: 'Weekly', used_percent: 70, resets_at: Math.floor(Date.now() / 1000) + 172800 }],
          summary: {
            lifetime_tokens: 7111812241,
            last_7d_tokens: 452818756,
            peak_daily_tokens: null,
            current_streak_days: 3,
            local_sessions: null,
            local_cost_usd: null,
            usage_usd: null,
            limit_remaining_usd: null,
          },
          action_url: null,
        },
        {
          id: 'claude',
          name: 'Claude Code',
          installed: true,
          connected: false,
          auth_label: 'Claude.ai OAuth',
          status_message: 'Open /usage in Claude Code for subscription limits.',
          support: 'manual',
          windows: [],
          summary: {
            lifetime_tokens: null,
            last_7d_tokens: null,
            peak_daily_tokens: null,
            current_streak_days: null,
            local_sessions: null,
            local_cost_usd: null,
            usage_usd: null,
            limit_remaining_usd: null,
          },
          action_url: null,
        },
        {
          id: 'opencode',
          name: 'OpenCode',
          installed: true,
          connected: true,
          auth_label: '4 OAuth providers',
          status_message: 'Local activity from opencode stats.',
          support: 'local',
          windows: [],
          summary: {
            lifetime_tokens: null,
            last_7d_tokens: null,
            peak_daily_tokens: null,
            current_streak_days: null,
            local_sessions: 18,
            local_cost_usd: 1.42,
            usage_usd: null,
            limit_remaining_usd: null,
          },
          action_url: null,
        },
        {
          id: 'openrouter',
          name: 'OpenRouter',
          installed: true,
          connected: false,
          auth_label: 'OAuth PKCE',
          status_message: 'No Zenith OAuth session is connected yet.',
          support: 'live',
          windows: [],
          summary: {
            lifetime_tokens: null,
            last_7d_tokens: null,
            peak_daily_tokens: null,
            current_streak_days: null,
            local_sessions: null,
            local_cost_usd: null,
            usage_usd: null,
            limit_remaining_usd: null,
          },
          action_url: null,
        },
        {
          id: 'antigravity',
          name: 'Antigravity',
          installed: true,
          connected: false,
          auth_label: 'Google OAuth',
          status_message: 'Google does not publish an account-usage API.',
          support: 'manual',
          windows: [],
          summary: {
            lifetime_tokens: null,
            last_7d_tokens: null,
            peak_daily_tokens: null,
            current_streak_days: null,
            local_sessions: null,
            local_cost_usd: null,
            usage_usd: null,
            limit_remaining_usd: null,
          },
          action_url: null,
        },
      ],
    };
  },

  async getAiControlCenter(_force = false): Promise<AiControlCenterSnapshot> {
    return mockControlSnapshot();
  },

  async getAiControlQuickSummary(): Promise<ControlCenterQuickSummary | null> {
    return mockControlSnapshot().quick_summary;
  },

  async saveAiControlPreferences(preferences: AiControlPreferences): Promise<void> {
    mockControlPreferences = structuredClone(preferences);
  },

  async runAiSafetyScan(): Promise<SafetySnapshot> {
    const snapshot = mockControlSnapshot().safety;
    return { ...snapshot, quality: 'fresh', scanned_files: 12, status_message: 'Bounded inspection complete.' };
  },

  async dismissAiSafetyFinding(_findingId: string): Promise<void> {},

  async previewAiRecommendation(recommendationId: string): Promise<RecommendationPreview> {
    return { id: `preview-${recommendationId}`, recommendation_id: recommendationId, title: 'Review open development port', explanation: 'This opens the existing Development Servers workflow. No process is changed by this preview.', destination: 'development_servers', action_label: 'Open Development Servers', expires_at: Math.floor(Date.now() / 1000) + 120 };
  },

  async consumeAiRecommendationPreview(previewId: string): Promise<RecommendationPreview> {
    if (!previewId.startsWith('preview-')) throw new Error('Preview expired or already used');
    return { id: previewId, recommendation_id: previewId.slice(8), title: 'Review', explanation: 'Validated once.', destination: 'development_servers', action_label: 'Open Development Servers', expires_at: Math.floor(Date.now() / 1000) + 120 };
  },

  async getAiControlGitDiff(_projectId: string): Promise<string> {
    return 'diff --git a/src/example.ts b/src/example.ts\n+// Explicit, ephemeral preview';
  },

  async connectOpenRouter(): Promise<void> {
    // No-op in browser mock
  },

  async startScan(
    onEvent: (event: ScanEvent) => void,
    _categories?: Category[]
  ): Promise<ScanResult> {
    return new Promise((resolve) => {
      const scanId = 'mock-scan-' + Date.now();
      onEvent({ type: 'Started', scan_id: scanId });

      setTimeout(() => {
        onEvent({ type: 'CategoryStarted', category: 'ai' });
        onEvent({
          type: 'ItemFound',
          item: {
            id: 'ai.cursor.cache',
            signature_id: 'ai.cursor.cache',
            name: 'Cursor Editor Cache',
            category: 'ai',
            risk: 'safe',
            path: '~/Library/Caches/Cursor',
            size: { logical: 2.1 * 1024 * 1024 * 1024, allocated: 2.1 * 1024 * 1024 * 1024 },
            file_count: 3200,
            description: 'V8 code cache and GPU shader cache',
            is_selected: true,
            last_modified: null,
            exists: true,
          },
        });
        onEvent({
          type: 'ItemFound',
          item: {
            id: 'ai.claude.logs',
            signature_id: 'ai.claude.logs',
            name: 'Claude Code Logs',
            category: 'ai',
            risk: 'safe',
            path: '~/.claude/logs',
            size: { logical: 1.1 * 1024 * 1024 * 1024, allocated: 1.1 * 1024 * 1024 * 1024 },
            file_count: 140,
            description: 'Session diagnostic logs',
            is_selected: true,
            last_modified: null,
            exists: true,
          },
        });
        onEvent({ type: 'CategoryFinished', category: 'ai', bytes: 3.2 * 1024 * 1024 * 1024, item_count: 2 });
      }, 150);

      setTimeout(() => {
        onEvent({ type: 'CategoryStarted', category: 'developer' });
        onEvent({
          type: 'ItemFound',
          item: {
            id: 'dev.go.build',
            signature_id: 'dev.go.build',
            name: 'Go Build Cache',
            category: 'developer',
            risk: 'safe',
            path: '~/Library/Caches/go-build',
            size: { logical: 3.1 * 1024 * 1024 * 1024, allocated: 3.1 * 1024 * 1024 * 1024 },
            file_count: 12000,
            description: 'Compiled packages cache',
            is_selected: true,
            last_modified: null,
            exists: true,
          },
        });
        onEvent({
          type: 'ItemFound',
          item: {
            id: 'dev.cargo.registry.cache',
            signature_id: 'dev.cargo.registry.cache',
            name: 'Cargo Registry Cache',
            category: 'developer',
            risk: 'rebuild',
            path: '~/.cargo/registry/cache',
            size: { logical: 2.0 * 1024 * 1024 * 1024, allocated: 2.0 * 1024 * 1024 * 1024 },
            file_count: 850,
            description: 'Downloaded crates archive',
            is_selected: false,
            last_modified: null,
            exists: true,
          },
        });
        onEvent({ type: 'CategoryFinished', category: 'developer', bytes: 5.1 * 1024 * 1024 * 1024, item_count: 2 });
      }, 300);

      setTimeout(() => {
        let intensiveCleanup = false;
        if (typeof localStorage !== 'undefined') {
          try {
            const saved = JSON.parse(localStorage.getItem('zenith.settings') ?? '{}');
            intensiveCleanup = saved.intensive_cleanup === true;
          } catch {
            intensiveCleanup = false;
          }
        }
        const intensiveBytes = 1.4 * 1024 * 1024 * 1024;
        const intensiveItem: ScanItem = {
          id: 'system.intensive.user_app_caches.mock-app',
          signature_id: 'system.intensive.user_app_caches',
          name: 'Stale Third-Party Application Cache (Mock App)',
          category: 'system',
          risk: 'safe',
          path: '~/Library/Caches/com.example.mock-app',
          size: { logical: intensiveBytes, allocated: intensiveBytes },
          file_count: 2400,
          description: 'Third-party cache inactive for at least 7 days',
          is_selected: true,
          last_modified: Math.floor(Date.now() / 1000) - 8 * 86400,
          exists: true,
        };
        const intensiveCategory: ScanResult['categories'][number] = {
          category: 'system',
          display_name: 'System',
          items: [intensiveItem],
          total_bytes: intensiveBytes,
          safe_bytes: intensiveBytes,
          rebuild_bytes: 0,
          manual_bytes: 0,
        };

        if (intensiveCleanup) {
          onEvent({ type: 'CategoryStarted', category: 'system' });
          onEvent({ type: 'ItemFound', item: intensiveItem });
          onEvent({
            type: 'CategoryFinished',
            category: 'system',
            bytes: intensiveBytes,
            item_count: 1,
          });
        }

        const result: ScanResult = {
          scan_id: scanId,
          started_at: Math.floor(Date.now() / 1000) - 1,
          finished_at: Math.floor(Date.now() / 1000),
          categories: [
            {
              category: 'ai',
              display_name: 'AI Tools',
              items: [
                {
                  id: 'ai.cursor.cache',
                  signature_id: 'ai.cursor.cache',
                  name: 'Cursor Editor Cache',
                  category: 'ai',
                  risk: 'safe',
                  path: '~/Library/Caches/Cursor',
                  size: { logical: 2.1 * 1024 * 1024 * 1024, allocated: 2.1 * 1024 * 1024 * 1024 },
                  file_count: 3200,
                  description: 'V8 code cache and GPU shader cache',
                  is_selected: true,
                  last_modified: null,
                  exists: true,
                },
                {
                  id: 'ai.claude.logs',
                  signature_id: 'ai.claude.logs',
                  name: 'Claude Code Logs',
                  category: 'ai',
                  risk: 'safe',
                  path: '~/.claude/logs',
                  size: { logical: 1.1 * 1024 * 1024 * 1024, allocated: 1.1 * 1024 * 1024 * 1024 },
                  file_count: 140,
                  description: 'Session diagnostic logs',
                  is_selected: true,
                  last_modified: null,
                  exists: true,
                },
              ],
              total_bytes: 3.2 * 1024 * 1024 * 1024,
              safe_bytes: 3.2 * 1024 * 1024 * 1024,
              rebuild_bytes: 0,
              manual_bytes: 0,
            },
            {
              category: 'developer',
              display_name: 'Developer',
              items: [
                {
                  id: 'dev.go.build',
                  signature_id: 'dev.go.build',
                  name: 'Go Build Cache',
                  category: 'developer',
                  risk: 'safe',
                  path: '~/Library/Caches/go-build',
                  size: { logical: 3.1 * 1024 * 1024 * 1024, allocated: 3.1 * 1024 * 1024 * 1024 },
                  file_count: 12000,
                  description: 'Compiled packages cache',
                  is_selected: true,
                  last_modified: null,
                  exists: true,
                },
                {
                  id: 'dev.cargo.registry.cache',
                  signature_id: 'dev.cargo.registry.cache',
                  name: 'Cargo Registry Cache',
                  category: 'developer',
                  risk: 'rebuild',
                  path: '~/.cargo/registry/cache',
                  size: { logical: 2.0 * 1024 * 1024 * 1024, allocated: 2.0 * 1024 * 1024 * 1024 },
                  file_count: 850,
                  description: 'Downloaded crates archive',
                  is_selected: false,
                  last_modified: null,
                  exists: true,
                },
              ],
              total_bytes: 5.1 * 1024 * 1024 * 1024,
              safe_bytes: 3.1 * 1024 * 1024 * 1024,
              rebuild_bytes: 2.0 * 1024 * 1024 * 1024,
              manual_bytes: 0,
            },
            ...(intensiveCleanup ? [intensiveCategory] : []),
          ],
          total_bytes: 8.3 * 1024 * 1024 * 1024 + (intensiveCleanup ? intensiveBytes : 0),
          safe_bytes: 6.3 * 1024 * 1024 * 1024 + (intensiveCleanup ? intensiveBytes : 0),
          rebuild_bytes: 2.0 * 1024 * 1024 * 1024,
          manual_bytes: 0,
        };

        onEvent({ type: 'Finished', result });
        resolve(result);
      }, 450);
    });
  },

  async getLastScan(): Promise<ScanResult | null> {
    return null;
  },

  async createPlan(_scanId: string, items: ScanItem[]): Promise<PlanPreview> {
    return {
      id: 'mock-plan-1',
      targets: items.map((i) => ({
        item_id: i.id,
        name: i.name,
        expected_bytes: i.size.allocated ?? i.size.logical,
        risk: i.risk,
      })),
      expected_reclaim_bytes: items.reduce(
        (acc, i) => acc + (i.size.allocated ?? i.size.logical),
        0
      ),
      risk: {
        safe_count: items.filter((i) => i.risk === 'safe').length,
        rebuild_count: items.filter((i) => i.risk === 'rebuild').length,
        manual_count: items.filter((i) => i.risk === 'manual').length,
        safe_bytes: items
          .filter((i) => i.risk === 'safe')
          .reduce((acc, i) => acc + (i.size.allocated ?? i.size.logical), 0),
        rebuild_bytes: items
          .filter((i) => i.risk === 'rebuild')
          .reduce((acc, i) => acc + (i.size.allocated ?? i.size.logical), 0),
        manual_bytes: items
          .filter((i) => i.risk === 'manual')
          .reduce((acc, i) => acc + (i.size.allocated ?? i.size.logical), 0),
      },
      expires_at: Math.floor(Date.now() / 1000) + 300,
    };
  },

  async executeClean(
    plan: PlanPreview,
    onEvent: (event: CleanEvent) => void
  ): Promise<CleanResult> {
    return new Promise((resolve) => {
      onEvent({
        type: 'Started',
        plan_id: plan.id,
        total_targets: plan.targets.length,
        expected_bytes: plan.expected_reclaim_bytes,
      });

      const items: CleanItemResult[] = [];
      plan.targets.forEach((t, i) => {
        setTimeout(() => {
          onEvent({
            type: 'ItemStarted',
            item_id: t.item_id,
            name: t.name,
            index: i + 1,
            total: plan.targets.length,
          });

          setTimeout(() => {
            onEvent({
              type: 'ItemFinished',
              item_id: t.item_id,
              name: t.name,
              success: true,
              reclaimed_bytes: t.expected_bytes,
              error: null,
            });
            items.push({
              item_id: t.item_id,
              name: t.name,
              path: '',
              status: 'success',
              success: true,
              bytes_reclaimed: t.expected_bytes,
              failure_reason: null,
              error_message: null,
            });

            if (items.length === plan.targets.length) {
              const res: CleanResult = {
                plan_id: plan.id,
                started_at: plan.expires_at - 300,
                finished_at: Math.floor(Date.now() / 1000),
                total_reclaimed_bytes: plan.expected_reclaim_bytes,
                total_failed_bytes: 0,
                items,
                actual_disk_free_delta: plan.expected_reclaim_bytes,
              };
              onEvent({ type: 'Finished', result: res });
              resolve(res);
            }
          }, 100);
        }, i * 200);
      });
    });
  },

  async getMemoryMetrics(): Promise<MemoryMetrics> {
    return {
      total_bytes: 16 * 1024 * 1024 * 1024,
      used_bytes: 11.8 * 1024 * 1024 * 1024,
      available_bytes: 4.2 * 1024 * 1024 * 1024,
      free_bytes: 1.2 * 1024 * 1024 * 1024,
      compressed_bytes: 2.1 * 1024 * 1024 * 1024,
      swap_used_bytes: 650 * 1024 * 1024,
      swap_total_bytes: 2 * 1024 * 1024 * 1024,
      pressure: 'normal',
      top_processes: [
        { pid: 102, pids: [102, 106, 107, 108], name: 'Google Chrome', memory_bytes: 6.9 * 1024 * 1024 * 1024, process_count: 109, can_terminate: true },
        { pid: 101, pids: [101, 109], name: 'Cursor', memory_bytes: 2.8 * 1024 * 1024 * 1024, process_count: 14, can_terminate: true },
        { pid: 103, pids: [103, 110, 111, 112], name: 'Docker Desktop', memory_bytes: 1.6 * 1024 * 1024 * 1024, process_count: 4, can_terminate: true },
        { pid: 104, pids: [104, 113], name: 'Claude', memory_bytes: 840 * 1024 * 1024, process_count: 2, can_terminate: true },
        { pid: 105, pids: [105, 114, 115], name: 'Xcode', memory_bytes: 1.4 * 1024 * 1024 * 1024, process_count: 6, can_terminate: true },
      ],
      timestamp: Math.floor(Date.now() / 1000),
    };
  },

  async terminateProcessGroup(name: string, _force: boolean): Promise<number> {
    return name === 'Google Chrome' ? 109 : 1;
  },

  async pickKeepAwakeApplication(): Promise<SelectedApplication | null> {
    return {
      name: 'Blender',
      executable_pattern: 'Blender',
      path: '/Applications/Blender.app',
    };
  },

  async getDiskMetrics(): Promise<DiskMetrics> {
    return {
      mount_point: '/',
      total_bytes: 494 * 1024 * 1024 * 1024,
      used_bytes: 341 * 1024 * 1024 * 1024,
      free_bytes: 153 * 1024 * 1024 * 1024,
      available_bytes: 153 * 1024 * 1024 * 1024,
      percent_used: 69.0,
    };
  },

  async getDiskVolumes(): Promise<DiskVolume[]> {
    return [
      {
        name: 'Macintosh HD',
        mount_point: '/',
        file_system: 'APFS',
        disk_type: 'SSD',
        total_bytes: 228.3 * 1024 * 1024 * 1024,
        used_bytes: 190.5 * 1024 * 1024 * 1024,
        available_bytes: 37.8 * 1024 * 1024 * 1024,
        percent_used: 83.5,
        is_removable: false,
        is_primary: true,
      },
      {
        name: 'Developer SSD',
        mount_point: '/Volumes/Developer SSD',
        file_system: 'APFS',
        disk_type: 'SSD',
        total_bytes: 1_000 * 1024 * 1024 * 1024,
        used_bytes: 642 * 1024 * 1024 * 1024,
        available_bytes: 358 * 1024 * 1024 * 1024,
        percent_used: 64.2,
        is_removable: true,
        is_primary: false,
      },
    ];
  },

  async openDiskUtility(): Promise<void> {
    // No-op in browser mock
  },

  async getDockerStatus(): Promise<DockerStatus> {
    return {
      is_available: true,
      is_running: true,
      version: 'Docker version 27.0.3',
      error_message: null,
      overview: {
        images: { total_bytes: 7.2 * 1024 * 1024 * 1024, reclaimable_bytes: 2.1 * 1024 * 1024 * 1024 },
        build_cache: { total_bytes: 8.1 * 1024 * 1024 * 1024, reclaimable_bytes: 8.1 * 1024 * 1024 * 1024 },
        containers: { total_bytes: 1.4 * 1024 * 1024 * 1024, reclaimable_bytes: 1.4 * 1024 * 1024 * 1024 },
        volumes: { total_bytes: 1.6 * 1024 * 1024 * 1024, reclaimable_bytes: 600 * 1024 * 1024 },
        total_bytes: 18.3 * 1024 * 1024 * 1024,
        total_reclaimable_bytes: 12.2 * 1024 * 1024 * 1024,
        safe_cleanable_bytes: 10.2 * 1024 * 1024 * 1024,
      },
      images: [],
      containers: [],
      volumes: [],
    };
  },

  async pruneDocker(_signatureId: string): Promise<number> {
    return 1024 * 1024 * 1024;
  },

  async getLocalModels(): Promise<LocalModelItem[]> {
    return [
      {
        id: 'ollama.llama3:70b',
        name: 'llama3:70b',
        source: 'ollama',
        path: '~/.ollama/models/manifests/registry.ollama.ai/library/llama3/70b',
        size_bytes: 18.2 * 1024 * 1024 * 1024,
        format: 'GGUF',
        parameter_size: null,
        quantization: null,
        last_modified: Math.floor(Date.now() / 1000) - 86400 * 3,
      },
      {
        id: 'ollama.qwen2.5-coder:32b',
        name: 'qwen2.5-coder:32b',
        source: 'ollama',
        path: '~/.ollama/models/manifests/registry.ollama.ai/library/qwen2.5-coder/32b',
        size_bytes: 9.8 * 1024 * 1024 * 1024,
        format: 'GGUF',
        parameter_size: null,
        quantization: null,
        last_modified: Math.floor(Date.now() / 1000) - 86400 * 1,
      },
      {
        id: 'hf.meta-llama/Llama-3.2-3B',
        name: 'meta-llama/Llama-3.2-3B',
        source: 'huggingface',
        path: '~/.cache/huggingface/hub/models--meta-llama--Llama-3.2-3B',
        size_bytes: 4.2 * 1024 * 1024 * 1024,
        format: 'safetensors',
        parameter_size: null,
        quantization: null,
        last_modified: Math.floor(Date.now() / 1000) - 86400 * 5,
      },
    ];
  },

  async deleteLocalModel(_modelId: string): Promise<number> {
    return 4.2 * 1024 * 1024 * 1024;
  },

  async getAwakeState(): Promise<AwakeState> {
    return {
      is_active: false,
      behavior: null,
      trigger_source: null,
      active_process_name: null,
      active_rule_id: null,
      manual_expires_at: null,
      active_rules_count: 2,
      power_source: 'ac',
      last_error: null,
      rule_evaluations: [
        {
          rule_id: 'rule.codex',
          status: 'waiting_process',
          is_process_running: false,
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
  },

  async setAwakeRules(_rules: AwakeRule[]): Promise<void> {
    // No-op in browser mock
  },

  async setManualAwake(
    _durationSecs: number | null,
    _behavior: AwakeBehavior
  ): Promise<void> {
    // No-op in browser mock
  },

  async disableManualAwake(): Promise<void> {
    // No-op in browser mock
  },

  async getSettings(): Promise<ZenithSettings_Serialize> {
    const raw = typeof localStorage !== 'undefined' ? localStorage.getItem('zenith.settings') : null;
    if (raw) {
      try {
        return JSON.parse(raw);
      } catch {
        // Fallback
      }
    }
    return {
      launch_at_login: false,
      clean_ai_tools: true,
      clean_developer_tools: true,
      clean_docker: true,
      clean_local_models: false,
      include_rebuild_caches: false,
      intensive_cleanup: false,
      theme: 'system',
      excluded_signatures: [],
      quick_panel_sections: ['cleanup', 'storage', 'memory', 'agent_activity'],
      quick_panel_ai_providers: ['codex', 'claude', 'opencode', 'openrouter', 'antigravity'],
      dashboard_tabs: ['storage', 'docker', 'models', 'memory', 'development_servers', 'projects', 'awake'],
      dashboard_tabs_revision: 5,
      sidebar_collapsed: false,
      awake_rules: [
        {
          id: 'rule.codex',
          app_name: 'Codex',
          executable_pattern: 'codex',
          behavior: 'prevent_system_sleep',
          power_condition: 'ac_power_only',
          enabled: false,
        },
        {
          id: 'rule.claude',
          app_name: 'Claude Code',
          executable_pattern: 'claude',
          behavior: 'prevent_system_sleep',
          power_condition: 'ac_power_only',
          enabled: false,
        },
      ],
      ai_control: mockControlPreferences,
      agent_notifications: {
        enabled: false,
        notify_on_turn_completed: true,
        notify_on_approval_or_input: true,
        notify_on_possibly_inactive: true,
        hide_project_basename: false,
        inactivity_threshold_minutes: 15,
      },
    };
  },

  async saveSettings(settings: ZenithSettings): Promise<void> {
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem('zenith.settings', JSON.stringify(settings));
    }
  },

  async revealInFinder(_path: string): Promise<void> {
    // No-op in browser mock
  },

  async openDashboard(): Promise<void> {
    if (typeof window !== 'undefined') {
      window.location.hash = '#dashboard';
    }
  },

  async toggleQuick(): Promise<void> {
    // No-op in browser mock
  },

  async getAppVersion(): Promise<string> {
    return typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : '0.1.0';
  },

  async getDiagnostics(): Promise<DiagnosticsSnapshot> {
    return {
      app_version: typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : '0.1.4',
      os_version: 'macOS 15.3.1 (Mock Preview)',
      arch: 'aarch64',
      log_path: '/Users/mock/Library/Logs/Zenith/zenith.log',
      enabled_features: [
        'dashboard_tabs: Storage, Docker, LocalModel, Memory, Projects, DevelopmentServers, AiUsage, Awake',
        'quick_panel_sections: Storage, Cleanup, AiUsage, Categories, Memory',
        'clean_categories: ai=true, dev=true, docker=false, models=false',
        'awake_rules: total=2, active=0',
      ],
      recent_errors: [],
      settings_corrupt_recovered: false,
    };
  },

  async openLogsFolder(): Promise<void> {
    // No-op in browser mock
  },

  async listDevelopmentListeners(): Promise<DevelopmentListener[]> {
    return [...mockListeners];
  },

  async releaseDevelopmentListener(
    id: string,
    mode: ReleaseMode
  ): Promise<ReleaseDevelopmentListenerResult> {
    const target = mockListeners.find((l) => l.id === id);
    if (!target) {
      throw new Error('Listener snapshot expired; refresh and try again.');
    }
    if (!target.can_release) {
      throw new Error('This listener is protected and cannot be released.');
    }

    if (target.port === 3000 && mode === 'graceful') {
      const freshLeaseId = `mock-lease-next-3000-force-${Date.now()}`;
      const freshListener: DevelopmentListener = {
        ...target,
        id: freshLeaseId,
      };
      mockListeners = mockListeners.map((l) => (l.id === id ? freshListener : l));
      return {
        port: target.port,
        outcome: 'still_listening',
        listener: freshListener,
      };
    }

    mockListeners = mockListeners.filter((l) => l.id !== id);
    return {
      port: target.port,
      outcome: 'released',
      listener: null,
    };
  },

  async hideCurrentWindow(): Promise<void> {
    // No-op in browser mock
  },
} satisfies ZenithApi;

let mockListeners: DevelopmentListener[] = [
  {
    id: 'mock-lease-vite-5173',
    port: 5173,
    protocol: 'tcp',
    bind_address: '127.0.0.1',
    exposure: 'loopback',
    pid: 32892,
    server_name: 'Vite',
    project_name: 'clean1',
    working_directory: '~/Myproject/clean1',
    started_at: Math.floor(Date.now() / 1000) - 17040,
    can_release: true,
    blocked_reason: null,
  },
  {
    id: 'mock-lease-next-3000',
    port: 3000,
    protocol: 'tcp',
    bind_address: '0.0.0.0',
    exposure: 'all_interfaces',
    pid: 40001,
    server_name: 'Next.js',
    project_name: 'web-dashboard',
    working_directory: '~/work/web-dashboard',
    started_at: Math.floor(Date.now() / 1000) - 1080,
    can_release: true,
    blocked_reason: null,
  },
  {
    id: 'mock-lease-pg-5432',
    port: 5432,
    protocol: 'tcp',
    bind_address: '127.0.0.1',
    exposure: 'loopback',
    pid: 5432,
    server_name: 'postgres',
    project_name: null,
    working_directory: null,
    started_at: Math.floor(Date.now() / 1000) - 86400,
    can_release: false,
    blocked_reason: 'Protected system, terminal, database, or container process',
  },
  {
    id: 'mock-lease-agent-browser-58937',
    port: 58937,
    protocol: 'tcp',
    bind_address: '127.0.0.1',
    exposure: 'loopback',
    pid: 88725,
    server_name: 'agent-browser',
    project_name: 'clean1',
    working_directory: '~/Myproject/clean1',
    started_at: Math.floor(Date.now() / 1000) - 120000,
    can_release: true,
    blocked_reason: null,
  },
  {
    id: 'mock-lease-chrome-testing-62850',
    port: 62850,
    protocol: 'tcp',
    bind_address: '127.0.0.1',
    exposure: 'loopback',
    pid: 24450,
    server_name: 'Chrome for Testing',
    project_name: 'clean1',
    working_directory: '~/Myproject/clean1',
    started_at: Math.floor(Date.now() / 1000) - 24000,
    can_release: true,
    blocked_reason: null,
  },
  {
    id: 'mock-lease-custom-8080',
    port: 8080,
    protocol: 'tcp',
    bind_address: '192.168.1.100',
    exposure: 'network',
    pid: 7777,
    server_name: 'worker-service',
    project_name: 'backend-services',
    working_directory: '~/backend-services',
    started_at: Math.floor(Date.now() / 1000) - 7200,
    can_release: false,
    blocked_reason: 'Not recognized as a development server',
  },
];
