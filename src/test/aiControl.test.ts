import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { render } from 'svelte/server';
import type {
  AiControlCenterSnapshot,
  AiControlPreferences,
  RecommendationPreview,
  SafetySnapshot,
} from '../lib/models/types';
import { AiControlStore, aiControlStore } from '../lib/stores/aiControl.svelte';
import AiControlCenterView from '../routes/dashboard/AiControlCenterView.svelte';

const sampleSnapshot: AiControlCenterSnapshot = {
  observed_at: 1700000000,
  providers: [
    {
      provider_id: 'codex-subscription',
      display_name: 'Codex subscription',
      source_kind: 'live_quota',
      source_id: 'codex-app-server',
      scope: 'subscription',
      observed_at: 1700000000,
      period: {
        starts_at: null,
        ends_at: 1700172800,
        resets_at: 1700172800,
        label: 'Weekly reset',
      },
      fresh_for_seconds: 300,
      quality: 'fresh',
      installed: true,
      connected: true,
      status_message: 'Live subscription limits from Codex.',
      metrics: [
        {
          label: 'Weekly quota',
          tokens: null,
          cost: null,
          used_basis_points: 7500,
        },
      ],
      action_url: null,
      partial_error: null,
    },
    {
      provider_id: 'opencode-local',
      display_name: 'OpenCode local activity',
      source_kind: 'local_estimate',
      source_id: 'opencode-stats',
      scope: 'local_sessions',
      observed_at: 1700000000,
      period: {
        starts_at: 1699395200,
        ends_at: 1700000000,
        resets_at: null,
        label: 'Last 7 days',
      },
      fresh_for_seconds: 300,
      quality: 'fresh',
      installed: true,
      connected: true,
      status_message: 'Local estimate; not a provider bill.',
      metrics: [
        {
          label: 'Local estimate',
          tokens: 2500000,
          cost: { micros: 3500000, currency: 'USD' },
          used_basis_points: null,
        },
      ],
      action_url: null,
      partial_error: null,
    },
    {
      provider_id: 'openai-api',
      display_name: 'OpenAI API organization',
      source_kind: 'manual',
      source_id: 'openai-api.capability',
      scope: 'organization',
      observed_at: 1700000000,
      period: {
        starts_at: null,
        ends_at: null,
        resets_at: null,
        label: 'Not connected',
      },
      fresh_for_seconds: 300,
      quality: 'unavailable',
      installed: false,
      connected: false,
      status_message: 'Separate from Codex subscription; optional managed credentials must use Keychain.',
      metrics: [],
      action_url: null,
      partial_error: null,
    },
    {
      provider_id: 'stale-provider',
      display_name: 'Stale Provider',
      source_kind: 'live_authoritative',
      source_id: 'stale.provider',
      scope: 'api_key',
      observed_at: 1699990000,
      period: {
        starts_at: null,
        ends_at: null,
        resets_at: null,
        label: 'Current observation',
      },
      fresh_for_seconds: 60,
      quality: 'stale',
      installed: true,
      connected: true,
      status_message: 'Last successful observation retained after refresh failure.',
      metrics: [
        {
          label: 'Key usage',
          tokens: null,
          cost: { micros: 12500000, currency: 'USD' },
          used_basis_points: null,
        },
      ],
      action_url: null,
      partial_error: 'Upstream rate limit',
    },
  ],
  budget_statuses: [
    {
      budget_id: 'budget-primary',
      period: 'monthly',
      spent: { micros: 40000000, currency: 'USD' },
      limit: { micros: 50000000, currency: 'USD' },
      used_basis_points: 8000,
      crossed_thresholds: [50, 80],
      source_label: 'Zenith alert budget',
      mixed_sources: true,
    },
  ],
  resources: [
    {
      session_id: 'session-verified-1',
      project_id: 'project-zenith',
      tool_name: 'Codex CLI',
      cpu_percent: 4.5,
      memory_bytes: 512 * 1024 * 1024,
      process_count: 1,
      duration_seconds: 1800,
      open_dev_ports: 2,
      power_eligible: true,
      confidence: 'process_observed',
      reason: 'Attributed through canonical project/session snapshot.',
      mutable_actions_allowed: true,
    },
    {
      session_id: 'session-orphan-2',
      project_id: null,
      tool_name: 'Claude Code',
      cpu_percent: 1.2,
      memory_bytes: 256 * 1024 * 1024,
      process_count: 1,
      duration_seconds: 600,
      open_dev_ports: 0,
      power_eligible: false,
      confidence: 'unassigned',
      reason: 'The agent process is verified, but project correlation cannot be proven.',
      mutable_actions_allowed: false,
    },
  ],
  recommendations: [
    {
      id: 'rec-1',
      kind: 'development_port',
      title: 'Review active dev ports',
      message: '2 verified listeners are active in this project.',
      created_at: 1700000000,
      cooldown_until: 1700000900,
      session_id: 'session-verified-1',
      project_id: 'project-zenith',
      action_label: 'Open Development Servers',
      destination: 'development_servers',
    },
  ],
  safety: {
    observed_at: 1700000000,
    quality: 'fresh',
    findings: [
      {
        id: 'finding-1',
        project_id: 'project-zenith',
        kind: 'secrets_exposure',
        severity: 'critical',
        evidence_type: 'OpenAI-style API key',
        adapter: 'local_secret_detector',
        relative_path: 'src/config.ts',
        line_start: 14,
        line_end: 14,
        observed_at: 1700000000,
        remediation: 'Remove exposed secret immediately.',
        dismissed: false,
        normalized_evidence: null,
      },
      {
        id: 'finding-2',
        project_id: 'project-zenith',
        kind: 'mcp_servers',
        severity: 'warning',
        evidence_type: 'MCP server configured',
        adapter: 'claude',
        relative_path: '.claude/settings.json',
        line_start: 5,
        line_end: 5,
        observed_at: 1700000000,
        remediation: 'Review server scope in owning tool.',
        dismissed: true,
        normalized_evidence: {
          server_name: 'test-server',
          scope: 'project',
          transport: 'stdio',
          permission_mode: null,
          sandbox_mode: null,
          command_basename: 'node',
          domain: null,
        },
      },
    ],
    scanned_files: 84,
    skipped_files: 2,
    status_message: 'Bounded local inspection completed.',
  },
  git_summaries: [
    {
      project_id: 'project-zenith',
      baseline_head: 'abc1234',
      current_head: 'abc1234',
      baseline_at: 1699998000,
      added: 1,
      modified: 3,
      deleted: 0,
      renamed: 0,
      untracked: 2,
      changed_paths: ['src/routes/dashboard/AiControlCenterView.svelte'],
      available: true,
      status_message: 'Changes compared to Zenith baseline.',
    },
  ],
  audit: [
    {
      id: 'audit-1',
      timestamp: 1700000000,
      event_kind: 'safety_scan',
      outcome: 'ok',
      project_ref: 'project-zenith',
      message: 'Bounded local inspection completed.',
    },
  ],
  quick_summary: {
    observed_at: 1700000000,
    active_sessions: 2,
    budget_alerts: 1,
    safety_findings: 1,
    quality: 'fresh',
  },
  keep_awake_active: false,
  partial_errors: [],
};

describe('AI Control Center Svelte component rendering', () => {
  beforeEach(() => {
    aiControlStore.snapshot = sampleSnapshot;
    aiControlStore.error = null;
    aiControlStore.isLoading = false;
    aiControlStore.preview = null;
    aiControlStore.gitDiff = null;
  });

  afterEach(() => {
    aiControlStore.snapshot = null;
    aiControlStore.error = null;
    aiControlStore.isLoading = false;
    aiControlStore.preview = null;
    aiControlStore.gitDiff = null;
  });

  it('renders all four section tabs and headers with provenance notice', () => {
    const rendered = render(AiControlCenterView);
    expect(rendered.body).toContain('AI Control Center');
    expect(rendered.body).toContain('Provenance-aware usage, verified sessions, and advisory safety controls');
    expect(rendered.body).toContain('Overview');
    expect(rendered.body).toContain('Usage &amp; Budgets');
    expect(rendered.body).toContain('Resource Autopilot');
    expect(rendered.body).toContain('Safety Posture');
    const source = readFileSync(
      path.resolve(process.cwd(), 'src/routes/dashboard/AiControlCenterView.svelte'),
      'utf-8'
    );
    expect(source).toContain('aria-label="Budget period"');
    expect(source).toContain('<option value="weekly">Weekly</option>');
  });

  it('renders overview metrics cards accurately', () => {
    const rendered = render(AiControlCenterView);
    expect(rendered.body).toContain('Observed sessions');
    expect(rendered.body).toContain('Provider sources');
    expect(rendered.body).toContain('Zenith alerts');
    expect(rendered.body).toContain('Safety findings');
    expect(rendered.body).toContain('Recent local audit');
    expect(rendered.body).toContain('Bounded local log · no telemetry · opaque project references');
  });

  it('renders recommendation items with action labels', () => {
    const rendered = render(AiControlCenterView);
    expect(rendered.body).toContain('Review active dev ports');
    expect(rendered.body).toContain('2 verified listeners are active in this project.');
    expect(rendered.body).toContain('Open Development Servers');
  });

  it('renders error notice when store has an error', () => {
    aiControlStore.error = 'Failed to load AI Control Center';
    const rendered = render(AiControlCenterView);
    expect(rendered.body).toContain('Failed to load AI Control Center');
  });

  it('renders partial snapshot banner when partial errors exist', () => {
    aiControlStore.snapshot = {
      ...sampleSnapshot,
      partial_errors: ['Provider rate limited', 'Symlink skipped'],
    };
    const rendered = render(AiControlCenterView);
    expect(rendered.body).toContain('Partial snapshot: Provider rate limited · Symlink skipped');
  });

  it('renders recommendation preview modal when a preview is active', () => {
    aiControlStore.preview = {
      id: 'prev-1',
      recommendation_id: 'rec-1',
      title: 'Review active dev ports',
      explanation: 'Opens Development Servers. No mutations performed.',
      destination: 'development_servers',
      action_label: 'Open Development Servers',
      expires_at: 1700000120,
    };
    const rendered = render(AiControlCenterView);
    expect(rendered.body).toContain('Review active dev ports');
    expect(rendered.body).toContain('Opens Development Servers. No mutations performed.');
    expect(rendered.body).toContain('One-shot preview expires automatically. No action has run.');
    expect(rendered.body).toContain('Open Development Servers');
  });

  it('renders ephemeral git diff modal when diff is loaded', () => {
    aiControlStore.gitDiff = 'diff --git a/file.ts b/file.ts\n+new line';
    const rendered = render(AiControlCenterView);
    expect(rendered.body).toContain('Ephemeral Git diff');
    expect(rendered.body).toContain('+new line');
  });
});

describe('AiControlStore logic and transitions', () => {
  it('manages loading and error states during refresh', async () => {
    const store = new AiControlStore();
    expect(store.isLoading).toBe(false);
    expect(store.snapshot).toBeNull();

    const tauriModule = await import('../lib/utils/tauri');
    const spy = vi.spyOn(tauriModule, 'tauriGetAiControlCenter').mockResolvedValueOnce(sampleSnapshot);

    await store.refresh();
    expect(store.snapshot).toEqual(sampleSnapshot);
    expect(store.isLoading).toBe(false);
    expect(store.error).toBeNull();
    expect(spy).toHaveBeenCalledWith(false);

    vi.spyOn(tauriModule, 'tauriGetAiControlCenter').mockRejectedValueOnce(new Error('Network offline'));
    await store.refresh(true);
    expect(store.snapshot).toEqual(sampleSnapshot);
    expect(store.error).toBe('Network offline');
  });

  it('updates local state when safety finding is dismissed', async () => {
    const store = new AiControlStore();
    store.snapshot = { ...sampleSnapshot };

    const tauriModule = await import('../lib/utils/tauri');
    vi.spyOn(tauriModule, 'tauriDismissAiSafetyFinding').mockResolvedValueOnce();

    await store.dismissFinding('finding-1');
    const dismissed = store.snapshot?.safety.findings.find((f) => f.id === 'finding-1');
    expect(dismissed?.dismissed).toBe(true);
  });

  it('consumes recommendation preview and clears local state with DashboardRoute', async () => {
    const store = new AiControlStore();
    store.preview = {
      id: 'preview-xyz',
      recommendation_id: 'rec-1',
      title: 'Review artifacts',
      explanation: 'Explain',
      action_label: 'Open Developer Artifacts',
      destination: 'developer_artifacts',
      expires_at: 100,
    };

    const tauriModule = await import('../lib/utils/tauri');
    vi.spyOn(tauriModule, 'tauriConsumeAiRecommendationPreview').mockResolvedValueOnce({
      id: 'preview-xyz',
      recommendation_id: 'rec-1',
      title: 'Review artifacts',
      explanation: 'Explain',
      action_label: 'Open Developer Artifacts',
      destination: 'developer_artifacts',
      expires_at: 100,
    });

    const destination = await store.consumePreview();
    expect(destination).toBe('developer_artifacts');
    expect(destination).not.toBe('Open Developer Artifacts');
    expect(store.preview).toBeNull();
  });

  it('loads and clears ephemeral git diff', async () => {
    const store = new AiControlStore();
    expect(store.gitDiff).toBeNull();

    const tauriModule = await import('../lib/utils/tauri');
    vi.spyOn(tauriModule, 'tauriGetAiControlGitDiff').mockResolvedValueOnce('diff --git a/test.ts\n+added');

    await store.loadGitDiff('project-zenith');
    expect(store.gitDiff).toContain('+added');

    store.clearGitDiff();
    expect(store.gitDiff).toBeNull();
  });

  it('contract test: recommendation preview preserves typed DashboardTab destination distinct from action_label', async () => {
    const store = new AiControlStore();
    const tauriModule = await import('../lib/utils/tauri');

    const mockPreview: RecommendationPreview = {
      id: 'preview-123',
      recommendation_id: 'rec-port',
      title: 'Review open development port',
      explanation: 'Opens development servers view without executing commands.',
      action_label: 'Open Development Servers',
      destination: 'development_servers',
      expires_at: 1700000120,
    };

    vi.spyOn(tauriModule, 'tauriPreviewAiRecommendation').mockResolvedValueOnce(mockPreview);
    vi.spyOn(tauriModule, 'tauriConsumeAiRecommendationPreview').mockResolvedValueOnce(mockPreview);

    await store.createPreview('rec-port');
    expect(store.preview).toEqual(mockPreview);
    expect(store.preview?.destination).toBe('development_servers');
    expect(store.preview?.action_label).toBe('Open Development Servers');

    const navigatedDestination = await store.consumePreview();
    expect(navigatedDestination).toBe('development_servers');
    expect(navigatedDestination).not.toBe('Open Development Servers');
  });
});

describe('Tauri Capabilities security boundaries for AI Control Center', () => {
  it('quick-panel capability grants only the cached quick summary command', () => {
    const quickCapPath = path.resolve(__dirname, '../../src-tauri/capabilities/quick.json');
    const quickJson = JSON.parse(readFileSync(quickCapPath, 'utf-8'));

    expect(quickJson.permissions).toContain('allow-get-ai-control-quick-summary');
    expect(quickJson.permissions).not.toContain('allow-get-ai-control-center');
    expect(quickJson.permissions).not.toContain('allow-save-ai-control-preferences');
    expect(quickJson.permissions).not.toContain('allow-run-ai-safety-scan');
    expect(quickJson.permissions).not.toContain('allow-dismiss-ai-safety-finding');
    expect(quickJson.permissions).not.toContain('allow-preview-ai-recommendation');
    expect(quickJson.permissions).not.toContain('allow-consume-ai-recommendation-preview');
    expect(quickJson.permissions).not.toContain('allow-get-ai-control-git-diff');
  });

  it('main capability includes all AI Control Center permissions', () => {
    const mainCapPath = path.resolve(__dirname, '../../src-tauri/capabilities/main.json');
    const mainJson = JSON.parse(readFileSync(mainCapPath, 'utf-8'));

    expect(mainJson.permissions).toContain('allow-get-ai-control-center');
    expect(mainJson.permissions).toContain('allow-get-ai-control-quick-summary');
    expect(mainJson.permissions).toContain('allow-save-ai-control-preferences');
    expect(mainJson.permissions).toContain('allow-run-ai-safety-scan');
    expect(mainJson.permissions).toContain('allow-dismiss-ai-safety-finding');
    expect(mainJson.permissions).toContain('allow-preview-ai-recommendation');
    expect(mainJson.permissions).toContain('allow-consume-ai-recommendation-preview');
    expect(mainJson.permissions).toContain('allow-get-ai-control-git-diff');
  });
});
