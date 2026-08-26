import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { render } from 'svelte/server';
import type { AgentActivitySnapshot } from '../lib/models/types';
import { AgentActivityStore, agentActivityStore } from '../lib/stores/agentActivity.svelte';
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
        repository_id: 'repo-one',
        is_worktree: true,
        branch: 'feature/one',
      },
      sessions: [
        {
          id: 'opaque-session',
          tool_id: 'codex',
          tool_name: 'Codex CLI',
          status: 'active',
          evidence: 'process_observed',
          observed_at: 100,
          started_at: 40,
          elapsed_seconds: 60,
          cpu_percent: 2.5,
          memory_bytes: 1024,
          project_id: 'project-one',
          detail: 'Process observed · detailed status unavailable',
        },
      ],
      last_seen_at: 100,
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
    },
  ],
  partial_errors: ['One adapter was unavailable.'],
};

afterEach(() => {
  agentActivityStore.snapshot = null;
  agentActivityStore.error = null;
  agentActivityStore.isLoading = false;
});

describe('Project Cockpit', () => {
  it('renders worktree identity and evidence without destructive controls', () => {
    agentActivityStore.snapshot = snapshot;
    const rendered = render(ProjectCockpitView);
    const source = readFileSync(new URL('../routes/dashboard/ProjectCockpitView.svelte', import.meta.url), 'utf8');

    expect(rendered.body).toContain('same-name');
    expect(rendered.body).toContain('parent-one/same-name');
    expect(rendered.body).toContain('Worktree');
    expect(rendered.body).toContain('Process observed');
    expect(rendered.body).toContain('Local only');
    expect(source).not.toContain('terminate_process');
    expect(source).not.toContain('requestStop');
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
});
