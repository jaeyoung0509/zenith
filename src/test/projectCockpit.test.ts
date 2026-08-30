import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { render } from 'svelte/server';
import type { AgentActivitySnapshot, AgentIntegrationInfo } from '../lib/models/types';
import { AgentActivityStore, agentActivityStore } from '../lib/stores/agentActivity.svelte';
import { usageStore } from '../lib/stores/usage.svelte';
import ProjectsPanel from '../lib/components/ai-activity/ProjectsPanel.svelte';
import ToolAdaptersPanel from '../lib/components/ai-activity/ToolAdaptersPanel.svelte';
import ProjectCockpitView from '../routes/dashboard/ProjectCockpitView.svelte';

const snapshot: AgentActivitySnapshot = {
  observed_at: 100,
  quality: 'partial',
  projects: [
    {
      identity: {
        id: 'project-one',
        display_name: 'same-name',
        location_hint: 'parent-one/same-name',
        display_path: '~/parent-one/same-name',
        repository_id: 'repo-one',
        worktree_id: 'worktree-one',
        is_worktree: true,
        branch: 'feature/one',
        is_dirty: false,
        is_detached: false,
      },
      sessions: [
        {
          id: 'opaque-session',
          tool_id: 'codex',
          tool_name: 'Codex CLI',
          status: 'active',
          attention_reason: null,
          evidence: 'process_observed',
          observed_at: 100,
          started_at: 40,
          elapsed_seconds: 60,
          cpu_percent: 2.5,
          memory_bytes: 1024,
          project_id: 'project-one',
          worktree_id: 'worktree-one',
          detail: 'Process observed · detailed status unavailable',
          can_stop: true,
          stop_lease_id: 'lease-test-1',
        },
      ],
      last_seen_at: 100,
      dev_ports: [5173],
      artifact_size_bytes: 1024 * 1024 * 50,
    },
  ],
  unassigned_sessions: [],
  adapters: [
    {
      tool_id: 'codex',
      display_name: 'Codex CLI',
      state: 'process_only',
      evidence: 'process_observed',
      message: 'Process observed · detailed status unavailable.',
      installed_version: '1.0.0',
    },
  ],
  partial_errors: ['One adapter was unavailable.'],
};

afterEach(() => {
  agentActivityStore.snapshot = null;
  agentActivityStore.selectedProjectId = null;
  agentActivityStore.error = null;
  agentActivityStore.isLoading = false;
  agentActivityStore.integrations = [];
  agentActivityStore.integrationsError = null;
  agentActivityStore.isIntegrationsLoading = false;
  usageStore.snapshot = null;
  usageStore.loadingProviders = [];
  usageStore.isLoading = false;
  usageStore.error = null;
});

describe('Project Cockpit', () => {
  it('renders worktree identity, dev ports, and evidence in project list', () => {
    agentActivityStore.snapshot = snapshot;
    const rendered = render(ProjectsPanel);
    const source = readFileSync(
      new URL('../lib/components/ai-activity/ProjectsPanel.svelte', import.meta.url),
      'utf8'
    );

    expect(rendered.body).toContain('same-name');
    expect(rendered.body).toContain('Worktree');
    expect(rendered.body).toContain('Process observed');
    expect(rendered.body).toContain(':5173');
    // Never calls raw arbitrary process kill
    expect(source).not.toContain('terminate_process_group');
    expect(source).not.toContain('kill -9');
  });

  it('renders Level 2 Project Cockpit when a project is selected', () => {
    agentActivityStore.snapshot = snapshot;
    agentActivityStore.selectProject('project-one');
    const rendered = render(ProjectCockpitView);

    expect(rendered.body).toContain('Back to Projects');
    expect(rendered.body).toContain('Active Agent Sessions');
    expect(rendered.body).toContain('Development Services');
    expect(rendered.body).toContain('Developer Storage');
    expect(rendered.body).toContain('localhost:5173');
    expect(rendered.body).toContain('Reveal in Finder');
    expect(rendered.body).toContain('Open in Terminal');
    expect(rendered.body).toContain('Stop');
  });

  it('defaults to Usage and exposes exactly three accessible sub-tabs', () => {
    agentActivityStore.snapshot = snapshot;
    const rendered = render(ProjectCockpitView);

    expect(rendered.body.match(/role="tab"/g)).toHaveLength(3);
    expect(rendered.body).toContain('role="tablist"');
    expect(rendered.body).toContain('id="ai-activity-tab-usage" role="tab" aria-selected="true"');
    expect(rendered.body).toContain('Usage');
    expect(rendered.body).toContain('Projects');
    expect(rendered.body).toContain('Tool Adapters');
    expect(rendered.body).not.toContain('Canonical projects');
    expect(rendered.body).not.toContain('Verified projects');
  });

  it('keeps the adapter matrix isolated from projects and usage content', () => {
    agentActivityStore.snapshot = snapshot;
    const rendered = render(ToolAdaptersPanel);

    expect(rendered.body).toContain('Tool Adapters');
    expect(rendered.body).toContain('Codex CLI');
    expect(rendered.body).toContain('Process observed');
    expect(rendered.body).not.toContain('AI Accounts &amp; Quota');
    expect(rendered.body).not.toContain('Canonical projects');
  });

  it('keeps the last successful snapshot when refresh fails', async () => {
    const fetch = vi.fn().mockResolvedValueOnce(snapshot).mockRejectedValueOnce(new Error('denied'));
    const store = new AgentActivityStore(fetch);

    await store.refresh();
    await store.refresh(true);

    expect(store.snapshot).toEqual(snapshot);
    expect(store.error).toBe('denied');
    expect(fetch).toHaveBeenLastCalledWith(true);
  });

  it('coalesces concurrent refreshes', async () => {
    let resolve!: (value: AgentActivitySnapshot) => void;
    const fetch = vi.fn(() => new Promise<AgentActivitySnapshot>((done) => (resolve = done)));
    const store = new AgentActivityStore(fetch);
    const first = store.refresh();
    const second = store.refresh();
    resolve(snapshot);
    await Promise.all([first, second]);
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it('coalesces concurrent integration lookups', async () => {
    let resolve!: (value: AgentIntegrationInfo[]) => void;
    const getIntegrations = vi.fn(
      () => new Promise<AgentIntegrationInfo[]>((done) => (resolve = done))
    );
    const store = new AgentActivityStore(vi.fn().mockResolvedValue(snapshot), getIntegrations);
    const first = store.fetchIntegrations();
    const second = store.fetchIntegrations();
    resolve([]);
    await Promise.all([first, second]);

    expect(getIntegrations).toHaveBeenCalledTimes(1);
  });

  it('handles graceful stop session delegation', async () => {
    const store = new AgentActivityStore(vi.fn().mockResolvedValue(snapshot));
    store.snapshot = snapshot;
    const stopSpy = vi.spyOn(store, 'stopSession').mockResolvedValue(undefined);

    await store.stopSession('opaque-session', 'lease-test-1');
    expect(stopSpy).toHaveBeenCalledWith('opaque-session', 'lease-test-1');
  });

  it('renders AI accounts quota strip when usage data is present', () => {
    agentActivityStore.snapshot = snapshot;
    usageStore.snapshot = {
      fetched_at: 100,
      providers: [
        {
          id: 'codex',
          name: 'Codex CLI',
          installed: true,
          connected: true,
          support: 'live',
          auth_label: 'CLI',
          status_message: 'Connected',
          action_url: null,
          windows: [
            { label: '5h limit', used_percent: 45, resets_at: 2_000_000_000 },
            { label: 'Weekly', used_percent: 21, resets_at: 2_000_100_000 },
          ],
          summary: {
            lifetime_tokens: 7_700_000_000,
            last_7d_tokens: 409_100_000,
            peak_daily_tokens: null,
            current_streak_days: 9,
            local_sessions: null,
            local_cost_usd: null,
            usage_usd: null,
            limit_remaining_usd: null,
          },
        },
      ],
    };
    const rendered = render(ProjectCockpitView);
    expect(rendered.body).toContain('AI Accounts');
    expect(rendered.body).toContain('Codex CLI');
    expect(rendered.body).toContain('45% used');
    expect(rendered.body).toContain('Weekly');
    expect(rendered.body).toContain('Lifetime');
    expect(rendered.body).toContain('Recent 7 days');
    expect(rendered.body).toContain('Resets');
  });

  it('renders stable provider card shells before streamed values arrive', () => {
    agentActivityStore.snapshot = snapshot;
    usageStore.isLoading = true;
    usageStore.loadingProviders = ['codex', 'claude', 'opencode', 'openrouter', 'antigravity'];

    const rendered = render(ProjectCockpitView);
    const providerNames = ['Codex', 'Claude Code', 'OpenCode', 'OpenRouter', 'Antigravity'];

    for (const name of providerNames) {
      expect(rendered.body).toContain(`Loading ${name} usage`);
    }
    expect(rendered.body.indexOf('Loading Codex usage')).toBeLessThan(
      rendered.body.indexOf('Loading Claude Code usage')
    );
    expect(rendered.body.indexOf('Loading Claude Code usage')).toBeLessThan(
      rendered.body.indexOf('Loading OpenCode usage')
    );
  });
});
