<script lang="ts">
  import { onMount } from 'svelte';
  import { Activity, Bot, FolderGit2, RefreshCw, ShieldCheck } from 'lucide-svelte';
  import type { AgentAdapterState, AgentEvidence, AgentSession } from '../../lib/models/types';
  import { agentActivityStore } from '../../lib/stores/agentActivity.svelte';
  import { formatBytes } from '../../lib/utils/format';
  import Badge from '../../lib/components/Badge.svelte';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';

  let snapshot = $derived(agentActivityStore.snapshot);

  onMount(() => {
    void agentActivityStore.refresh();
  });

  function evidenceLabel(evidence: AgentEvidence) {
    if (evidence === 'vendor_confirmed') return 'Vendor confirmed';
    if (evidence === 'heuristic') return 'Heuristic';
    return 'Process observed';
  }

  function adapterLabel(value: AgentAdapterState) {
    return value.split('_').map((part) => part[0].toUpperCase() + part.slice(1)).join(' ');
  }

  function duration(seconds: number) {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    return hours > 0 ? `${hours}h ${minutes}m` : `${Math.max(minutes, 1)}m`;
  }

  function sessionMetrics(session: AgentSession) {
    return `${session.cpu_percent?.toFixed(1) ?? '—'}% CPU · ${formatBytes(session.memory_bytes)} · ${duration(session.elapsed_seconds)}`;
  }
</script>

<div class="space-y-6 max-w-5xl">
  <header class="flex items-start justify-between gap-4 border-b border-border/60 pb-4">
    <div class="flex items-center gap-3 min-w-0">
      <div class="h-9 w-9 shrink-0 rounded-lg bg-secondary text-foreground flex items-center justify-center">
        <FolderGit2 size={19} />
      </div>
      <div class="min-w-0">
        <div class="flex items-center gap-2">
          <h2 class="text-base font-semibold tracking-tight">Projects</h2>
          <Badge variant="outline">Local only</Badge>
        </div>
        <p class="mt-0.5 text-xs text-muted-foreground">
          Active AI tools grouped only when their process identity and project context can be verified.
        </p>
      </div>
    </div>
    <Button
      variant="outline"
      size="sm"
      disabled={agentActivityStore.isLoading}
      ariaLabel="Refresh project activity"
      title="Refresh project activity"
      onclick={() => agentActivityStore.refresh(true)}
    >
      <RefreshCw size={13} class={agentActivityStore.isLoading ? 'animate-spin' : ''} />
      {agentActivityStore.isLoading ? 'Refreshing' : 'Refresh'}
    </Button>
  </header>

  {#if agentActivityStore.error}
    <div role="alert" class="rounded-xl border border-destructive/20 bg-destructive/5 px-4 py-3 text-xs text-destructive">
      Refresh failed. The last successful local snapshot remains visible. {agentActivityStore.error}
    </div>
  {/if}

  {#if !snapshot && agentActivityStore.isLoading}
    <div aria-label="Loading project activity" class="space-y-3">
      {#each Array(3) as _}
        <div class="h-28 animate-pulse rounded-xl border border-border/60 bg-secondary/30"></div>
      {/each}
    </div>
  {:else if snapshot}
    <section aria-label="Project activity summary" class="grid grid-cols-3 gap-3">
      <Card class="p-4 bg-card/70">
        <div class="text-meta text-muted-foreground">Verified projects</div>
        <div class="mt-1 text-xl font-semibold tabular-nums">{snapshot.projects.length}</div>
      </Card>
      <Card class="p-4 bg-card/70">
        <div class="text-meta text-muted-foreground">Observed sessions</div>
        <div class="mt-1 text-xl font-semibold tabular-nums">{agentActivityStore.activeSessionCount}</div>
      </Card>
      <Card class="p-4 bg-card/70">
        <div class="text-meta text-muted-foreground">Snapshot quality</div>
        <div class="mt-2"><Badge variant={snapshot.quality === 'fresh' ? 'success' : 'warning'}>{snapshot.quality}</Badge></div>
      </Card>
    </section>

    {#if snapshot.projects.length === 0 && snapshot.unassigned_sessions.length === 0}
      <Card class="py-12 px-6 text-center bg-card/60">
        <Bot size={26} class="mx-auto text-muted-foreground" />
        <h3 class="mt-3 text-sm font-medium">No active agent sessions</h3>
        <p class="mx-auto mt-1 max-w-md text-xs leading-relaxed text-muted-foreground">
          Start a supported CLI inside a project. Zenith observes local processes only and does not read prompts, arguments, transcripts, or credentials.
        </p>
      </Card>
    {:else}
      <section aria-label="Canonical projects" class="space-y-3">
        {#each snapshot.projects as project (project.identity.id)}
          <Card class="p-4 bg-card/70">
            <div class="flex items-start justify-between gap-4">
              <div class="min-w-0">
                <div class="flex items-center gap-2 min-w-0">
                  <h3 class="truncate text-sm font-semibold">{project.identity.display_name}</h3>
                  {#if project.identity.is_worktree}<Badge variant="outline">Worktree</Badge>{/if}
                </div>
                <p class="mt-0.5 truncate font-mono text-caption text-muted-foreground">
                  {project.identity.location_hint}{project.identity.branch ? ` · ${project.identity.branch}` : ''}
                </p>
              </div>
              <Badge variant="success">{project.sessions.length} active</Badge>
            </div>

            <div class="mt-4 divide-y divide-border/50 rounded-lg border border-border/60 bg-background/35">
              {#each project.sessions as session (session.id)}
                <div class="flex items-center gap-3 px-3 py-2.5">
                  <div class="h-7 w-7 shrink-0 rounded-md bg-secondary flex items-center justify-center text-muted-foreground"><Bot size={14} /></div>
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2">
                      <span class="truncate text-xs font-medium">{session.tool_name}</span>
                      <Badge variant="outline">{evidenceLabel(session.evidence)}</Badge>
                    </div>
                    <p class="mt-0.5 truncate text-caption text-muted-foreground">{session.detail}</p>
                  </div>
                  <div class="shrink-0 text-right font-mono text-caption text-muted-foreground" aria-label={sessionMetrics(session)}>
                    {sessionMetrics(session)}
                  </div>
                </div>
              {/each}
            </div>
          </Card>
        {/each}
      </section>
    {/if}

    {#if snapshot.unassigned_sessions.length > 0}
      <section aria-labelledby="unassigned-heading" class="space-y-2">
        <div>
          <h3 id="unassigned-heading" class="text-xs font-semibold">Unassigned sessions</h3>
          <p class="text-caption text-muted-foreground">These processes were verified, but their project could not be proven.</p>
        </div>
        <Card class="divide-y divide-border/50 bg-card/70">
          {#each snapshot.unassigned_sessions as session (session.id)}
            <div class="flex items-center justify-between gap-3 px-4 py-3">
              <div><span class="text-xs font-medium">{session.tool_name}</span><p class="text-caption text-muted-foreground">{session.detail}</p></div>
              <Badge variant="warning">Unassigned</Badge>
            </div>
          {/each}
        </Card>
      </section>
    {/if}

    <details class="rounded-xl border border-border/60 bg-card/50 p-4">
      <summary class="cursor-pointer list-none flex items-center gap-2 text-xs font-medium focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring rounded">
        <ShieldCheck size={14} /> Adapter health <span class="text-muted-foreground">({snapshot.adapters.length})</span>
      </summary>
      <div class="mt-3 grid grid-cols-2 gap-2">
        {#each snapshot.adapters as adapter (adapter.tool_id)}
          <div class="rounded-lg border border-border/50 px-3 py-2">
            <div class="flex items-center justify-between gap-2"><span class="truncate text-xs font-medium">{adapter.display_name}</span><Badge variant="outline">{adapterLabel(adapter.state)}</Badge></div>
            <p class="mt-1 text-caption leading-relaxed text-muted-foreground">{adapter.message}</p>
          </div>
        {/each}
      </div>
    </details>

    {#if snapshot.partial_errors.length > 0}
      <div class="rounded-xl border border-warning/20 bg-warning/5 px-4 py-3 text-xs text-warning" role="status">
        {snapshot.partial_errors.join(' ')}
      </div>
    {/if}
  {/if}
</div>
