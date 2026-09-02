<script lang="ts">
  import { Bot, HardDrive, Server, Square } from 'lucide-svelte';
  import type { AgentActivityStatus, AgentEvidence, AgentSession, AttentionReason } from '../../models/types';
  import { agentActivityStore } from '../../stores/agentActivity.svelte';
  import { formatBytes } from '../../utils/format';
  import Badge from '../Badge.svelte';
  import Button from '../Button.svelte';
  import Card from '../Card.svelte';

  let snapshot = $derived(agentActivityStore.snapshot);
  let stoppingSessionIds = $state<Set<string>>(new Set());
  let actionFeedback = $state<string | null>(null);

  function evidenceBadge(evidence: AgentEvidence) {
    if (evidence === 'vendor_confirmed' || evidence === 'vendor_event' || evidence === 'vendor_protocol') {
      return { label: 'Vendor confirmed', variant: 'success' as const };
    }
    if (evidence === 'heuristic') {
      return { label: 'Heuristic', variant: 'warning' as const };
    }
    return { label: 'Process observed', variant: 'secondary' as const };
  }

  function statusBadge(status: AgentActivityStatus) {
    switch (status) {
      case 'working':
      case 'active':
        return { label: 'Working', variant: 'success' as const };
      case 'waiting_for_user':
      case 'waiting':
        return { label: 'Waiting for User', variant: 'warning' as const };
      case 'starting':
        return { label: 'Starting', variant: 'secondary' as const };
      case 'idle':
        return { label: 'Idle', variant: 'secondary' as const };
      case 'possibly_inactive':
        return { label: 'Possibly Inactive', variant: 'warning' as const };
      case 'exited':
      case 'finished':
        return { label: 'Exited', variant: 'outline' as const };
      default:
        return { label: 'Unknown', variant: 'outline' as const };
    }
  }

  function attentionBadge(reason: AttentionReason | null) {
    if (!reason) return null;
    switch (reason) {
      case 'approval':
        return { label: 'Approval Needed', variant: 'danger' as const };
      case 'input':
        return { label: 'Input Needed', variant: 'warning' as const };
      case 'turn_complete':
        return { label: 'Turn Complete', variant: 'success' as const };
      case 'inactivity':
        return { label: 'Possibly Inactive', variant: 'warning' as const };
    }
  }

  function duration(seconds: number) {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    return hours > 0 ? `${hours}h ${minutes}m` : `${Math.max(minutes, 1)}m`;
  }

  function selectProject(id: string) {
    agentActivityStore.selectProject(id);
  }

  async function handleStopSession(session: AgentSession) {
    if (!session.can_stop || !session.stop_lease_id) return;
    stoppingSessionIds.add(session.id);
    stoppingSessionIds = new Set(stoppingSessionIds);
    try {
      await agentActivityStore.stopSession(session.id, session.stop_lease_id);
      actionFeedback = `Sent graceful stop (SIGTERM) to ${session.tool_name}.`;
    } catch (err) {
      actionFeedback = `Stop failed: ${err instanceof Error ? err.message : String(err)}`;
    } finally {
      stoppingSessionIds.delete(session.id);
      stoppingSessionIds = new Set(stoppingSessionIds);
      setTimeout(() => {
        actionFeedback = null;
      }, 5000);
    }
  }
</script>

<div
  id="ai-activity-panel-projects"
  role="tabpanel"
  aria-labelledby="ai-activity-tab-projects"
  tabindex="0"
  class="space-y-5 outline-none focus-visible:ring-1 focus-visible:ring-ring"
>
  {#if agentActivityStore.error}
    <div role="alert" class="rounded-xl border border-destructive/20 bg-destructive/5 px-4 py-3 text-xs text-destructive">
      Project refresh failed. The last successful local snapshot remains visible. {agentActivityStore.error}
    </div>
  {/if}

  {#if !snapshot && agentActivityStore.isLoading}
    <div aria-label="Loading project activity" class="space-y-3">
      {#each Array(3) as _}
        <div class="h-28 animate-pulse rounded-xl border border-border/60 bg-secondary/30"></div>
      {/each}
    </div>
  {:else if snapshot}
    {#if snapshot.partial_errors.length > 0}
      <div role="status" class="rounded-xl border border-warning/20 bg-warning/5 px-4 py-3 text-xs text-warning">
        Partial project snapshot: {snapshot.partial_errors.join(' · ')}
      </div>
    {/if}

    <section aria-label="Project activity summary" class="grid grid-cols-2 gap-3 sm:grid-cols-4">
      <Card class="p-4 bg-card/70">
        <div class="text-caption text-muted-foreground font-medium uppercase tracking-wider">Verified projects</div>
        <div class="mt-1 text-2xl font-bold tabular-nums">{snapshot.projects.length}</div>
      </Card>
      <Card class="p-4 bg-card/70">
        <div class="text-caption text-muted-foreground font-medium uppercase tracking-wider">Active sessions</div>
        <div class="mt-1 text-2xl font-bold tabular-nums text-foreground">{agentActivityStore.activeSessionCount}</div>
      </Card>
      <Card class="p-4 bg-card/70">
        <div class="text-caption text-muted-foreground font-medium uppercase tracking-wider">Attention needed</div>
        <div class="mt-1 text-2xl font-bold tabular-nums {agentActivityStore.attentionSessionCount > 0 ? 'text-destructive' : 'text-foreground'}">
          {agentActivityStore.attentionSessionCount}
        </div>
      </Card>
      <Card class="p-4 bg-card/70">
        <div class="text-caption text-muted-foreground font-medium uppercase tracking-wider">Snapshot quality</div>
        <div class="mt-2">
          <Badge variant={snapshot.quality === 'fresh' ? 'success' : 'warning'}>{snapshot.quality}</Badge>
        </div>
      </Card>
    </section>

    {#if snapshot.projects.length === 0 && snapshot.unassigned_sessions.length === 0}
      <Card class="py-12 px-6 text-center bg-card/60">
        <Bot size={28} class="mx-auto text-muted-foreground" />
        <h3 class="mt-3 text-sm font-semibold">No active agent sessions</h3>
        <p class="mx-auto mt-1.5 max-w-md text-xs leading-relaxed text-muted-foreground">
          Start a supported CLI (Antigravity, Claude Code, Cursor, Grok, Copilot, Codex, OpenCode) inside a project.
          Zenith observes local processes only and never reads prompts, transcripts, or credentials.
        </p>
      </Card>
    {:else}
      <section aria-label="Canonical projects" class="space-y-4">
        {#each snapshot.projects as project (project.identity.id)}
          {@const hasAttention = project.sessions.some((session) => session.attention_reason != null)}
          <Card class="p-5 bg-card/70 border-border/70 transition-colors hover:border-primary/40">
            <div class="flex items-start justify-between gap-4">
              <button
                type="button"
                class="min-w-0 flex-1 text-left rounded-md focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                aria-label={`Open project ${project.identity.display_name}`}
                onclick={() => selectProject(project.identity.id)}
              >
                <div class="flex items-center gap-2.5 min-w-0 flex-wrap">
                  <h3 class="truncate text-base font-bold tracking-tight">{project.identity.display_name}</h3>
                  {#if project.identity.is_worktree}
                    <Badge variant="outline">Worktree</Badge>
                  {/if}
                  {#if project.identity.branch}
                    <Badge variant="secondary" class="font-mono text-caption">{project.identity.branch}</Badge>
                  {/if}
                  {#if project.identity.is_detached}
                    <Badge variant="warning">Detached</Badge>
                  {/if}
                  {#if project.identity.is_dirty}
                    <Badge variant="warning">Modified</Badge>
                  {/if}
                  {#if hasAttention}
                    <Badge variant="danger">Needs Attention</Badge>
                  {/if}
                </div>
                <p class="mt-1 font-mono text-caption text-muted-foreground truncate">
                  {project.identity.display_path || project.identity.location_hint}
                </p>
              </button>

              <div class="flex items-center gap-2 shrink-0">
                <Badge variant={project.sessions.length > 0 ? 'success' : 'secondary'}>
                  {project.sessions.length} {project.sessions.length === 1 ? 'agent' : 'agents'}
                </Badge>
              </div>
            </div>

            <div class="mt-3 flex items-center gap-2 flex-wrap text-caption text-muted-foreground">
              {#if project.dev_ports.length > 0}
                <span class="inline-flex items-center gap-1 bg-secondary/80 px-2 py-0.5 rounded font-mono text-foreground">
                  <Server size={10} />
                  {project.dev_ports.map((port) => `:${port}`).join(', ')}
                </span>
              {/if}
              {#if project.artifact_size_bytes}
                <span class="inline-flex items-center gap-1 bg-secondary/80 px-2 py-0.5 rounded font-mono text-foreground">
                  <HardDrive size={10} />
                  {formatBytes(project.artifact_size_bytes)}
                </span>
              {/if}
            </div>

            {#if project.sessions.length > 0}
              <div class="mt-4 divide-y divide-border/50 rounded-lg border border-border/60 bg-background/40">
                {#each project.sessions as session (session.id)}
                  {@const status = statusBadge(session.status)}
                  {@const evidence = evidenceBadge(session.evidence)}
                  {@const attention = attentionBadge(session.attention_reason)}
                  <div class="flex items-center justify-between gap-3 px-3.5 py-2.5">
                    <div class="flex items-center gap-2.5 min-w-0">
                      <Bot size={14} class="text-muted-foreground shrink-0" />
                      <span class="text-xs font-semibold truncate">{session.tool_name}</span>
                      <Badge variant={status.variant}>{status.label}</Badge>
                      <Badge variant={evidence.variant}>{evidence.label}</Badge>
                      {#if attention}
                        <Badge variant={attention.variant}>{attention.label}</Badge>
                      {/if}
                      <span class="text-caption text-muted-foreground truncate">{session.detail}</span>
                    </div>
                    <div class="font-mono text-caption text-muted-foreground shrink-0">
                      {duration(session.elapsed_seconds)}
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          </Card>
        {/each}
      </section>
    {/if}

    {#if snapshot.unassigned_sessions.length > 0}
      <section aria-labelledby="unassigned-heading" class="space-y-3 pt-2">
        <div>
          <h3 id="unassigned-heading" class="text-sm font-semibold">Unassigned Agent Sessions</h3>
          <p class="text-caption text-muted-foreground">
            These processes were verified as allowlisted agents, but could not be correlated to a known project repository root.
          </p>
        </div>
        <Card class="divide-y divide-border/50 bg-card/70 border-border/70">
          {#each snapshot.unassigned_sessions as session (session.id)}
            {@const status = statusBadge(session.status)}
            {@const evidence = evidenceBadge(session.evidence)}
            <div class="flex items-center justify-between gap-3 px-4 py-3">
              <div class="min-w-0">
                <div class="flex items-center gap-2">
                  <span class="text-xs font-semibold">{session.tool_name}</span>
                  <Badge variant={status.variant}>{status.label}</Badge>
                  <Badge variant={evidence.variant}>{evidence.label}</Badge>
                </div>
                <p class="mt-0.5 text-caption text-muted-foreground">{session.detail}</p>
              </div>
              {#if session.can_stop}
                <Button
                  variant="destructive"
                  size="sm"
                  disabled={stoppingSessionIds.has(session.id)}
                  onclick={() => handleStopSession(session)}
                  title="Send graceful SIGTERM to agent process"
                >
                  <Square size={10} class="fill-current" />
                  {stoppingSessionIds.has(session.id) ? 'Stopping...' : 'Stop'}
                </Button>
              {/if}
            </div>
          {/each}
        </Card>
      </section>
    {/if}
  {:else}
    <Card class="py-12 px-6 text-center bg-card/60">
      <Bot size={28} class="mx-auto text-muted-foreground" />
      <h3 class="mt-3 text-sm font-semibold">No project activity snapshot</h3>
      <p class="mx-auto mt-1.5 max-w-md text-xs leading-relaxed text-muted-foreground">
        Zenith will show verified local project and agent activity after the first refresh.
      </p>
    </Card>
  {/if}

  {#if actionFeedback}
    <div role="status" class="rounded-xl border border-primary/20 bg-primary/5 px-4 py-3 text-xs text-primary">
      {actionFeedback}
    </div>
  {/if}
</div>
