<script lang="ts">
  import {
    ArrowLeft,
    Bot,
    ExternalLink,
    FolderOpen,
    HardDrive,
    RefreshCw,
    Server,
    Square,
    Terminal,
  } from 'lucide-svelte';
  import type {
    AgentActivityStatus,
    AgentEvidence,
    AgentSession,
    AttentionReason,
    ProjectContext,
  } from '../../lib/models/types';
  import { agentActivityStore } from '../../lib/stores/agentActivity.svelte';
  import { formatBytes } from '../../lib/utils/format';
  import { tauriRevealInFinder } from '../../lib/utils/tauri';
  import Badge from '../../lib/components/Badge.svelte';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';

  interface Props {
    project: ProjectContext;
    onBack: () => void;
    onNavigateTab?: (tab: string) => void;
  }

  let { project, onBack, onNavigateTab }: Props = $props();
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

  function handleRevealInFinder(path: string) {
    void tauriRevealInFinder(path);
  }

  function handleOpenInTerminal(path: string) {
    void agentActivityStore.openInTerminal(path);
  }
</script>

<div class="space-y-6">
  <div class="flex items-center justify-between gap-4">
    <Button
      variant="ghost"
      size="sm"
      onclick={onBack}
      class="gap-1.5 -ml-2 text-muted-foreground hover:text-foreground"
      ariaLabel="Back to Projects"
      title="Back to Projects"
    >
      <ArrowLeft size={14} />
      Back to Projects
    </Button>
    <div class="flex items-center gap-2">
      <Button
        variant="outline"
        size="sm"
        onclick={() => handleRevealInFinder(project.identity.display_path)}
        title="Reveal repository folder in Finder"
      >
        <FolderOpen size={13} />
        Reveal in Finder
      </Button>
      <Button
        variant="outline"
        size="sm"
        onclick={() => handleOpenInTerminal(project.identity.display_path)}
        title="Open repository folder in Terminal"
      >
        <Terminal size={13} />
        Open in Terminal
      </Button>
      <Button
        variant="outline"
        size="sm"
        disabled={agentActivityStore.isLoading}
        onclick={() => agentActivityStore.refresh(true)}
        title="Refresh project activity"
        ariaLabel="Refresh project activity"
      >
        <RefreshCw size={13} class={agentActivityStore.isLoading ? 'animate-spin' : ''} />
        Refresh
      </Button>
    </div>
  </div>

  <Card class="p-5 bg-card/70 border-border/70">
    <div class="flex flex-col md:flex-row md:items-center justify-between gap-4">
      <div>
        <div class="flex items-center gap-2.5 flex-wrap">
          <h2 class="text-xl font-bold tracking-tight">{project.identity.display_name}</h2>
          {#if project.identity.is_worktree}
            <Badge variant="outline">Worktree</Badge>
          {/if}
          {#if project.identity.branch}
            <Badge variant="secondary" class="font-mono">{project.identity.branch}</Badge>
          {/if}
          {#if project.identity.is_detached}
            <Badge variant="warning">Detached HEAD</Badge>
          {/if}
          {#if project.identity.is_dirty}
            <Badge variant="warning">Modified working tree</Badge>
          {/if}
        </div>
        <p class="mt-1 font-mono text-xs text-muted-foreground">{project.identity.display_path}</p>
      </div>

      <div class="flex items-center gap-4 text-xs text-muted-foreground divide-x divide-border/60">
        <div>
          <span class="font-semibold text-foreground">{project.sessions.length}</span>
          <span> agent sessions</span>
        </div>
        {#if project.dev_ports.length > 0}
          <div class="pl-4">
            <span class="font-semibold text-foreground">{project.dev_ports.length}</span>
            <span> dev ports</span>
          </div>
        {/if}
        {#if project.artifact_size_bytes}
          <div class="pl-4">
            <span class="font-semibold text-foreground">{formatBytes(project.artifact_size_bytes)}</span>
            <span> artifacts</span>
          </div>
        {/if}
      </div>
    </div>
  </Card>

  {#if actionFeedback}
    <div role="status" class="rounded-xl border border-primary/20 bg-primary/5 px-4 py-3 text-xs text-primary">
      {actionFeedback}
    </div>
  {/if}

  <div class="space-y-3">
    <div class="flex items-center justify-between">
      <h3 class="text-sm font-semibold tracking-tight">Active Agent Sessions</h3>
      <span class="text-caption text-muted-foreground">Graceful termination sends SIGTERM only</span>
    </div>

    {#if project.sessions.length === 0}
      <Card class="p-8 text-center bg-card/50">
        <Bot size={24} class="mx-auto text-muted-foreground mb-2" />
        <p class="text-xs text-muted-foreground">No active agent sessions running in this project.</p>
      </Card>
    {:else}
      <div class="space-y-3">
        {#each project.sessions as session (session.id)}
          {@const status = statusBadge(session.status)}
          {@const evidence = evidenceBadge(session.evidence)}
          {@const attention = attentionBadge(session.attention_reason)}
          <Card class="p-4 bg-card/70 border-border/70">
            <div class="flex items-start justify-between gap-3">
              <div class="flex items-start gap-3 min-w-0">
                <div class="h-8 w-8 shrink-0 rounded-lg bg-secondary flex items-center justify-center text-foreground mt-0.5">
                  <Bot size={16} />
                </div>
                <div class="min-w-0">
                  <div class="flex items-center gap-2 flex-wrap">
                    <span class="text-sm font-semibold">{session.tool_name}</span>
                    <Badge variant={status.variant}>{status.label}</Badge>
                    <Badge variant={evidence.variant}>{evidence.label}</Badge>
                    {#if attention}
                      <Badge variant={attention.variant}>{attention.label}</Badge>
                    {/if}
                  </div>
                  <p class="mt-1 text-xs text-muted-foreground">{session.detail}</p>
                </div>
              </div>

              {#if session.can_stop}
                <Button
                  variant="destructive"
                  size="sm"
                  disabled={stoppingSessionIds.has(session.id)}
                  onclick={() => handleStopSession(session)}
                  title="Send graceful SIGTERM to agent process"
                >
                  <Square size={11} class="fill-current" />
                  {stoppingSessionIds.has(session.id) ? 'Stopping...' : 'Stop'}
                </Button>
              {/if}
            </div>

            <div class="mt-4 pt-3 border-t border-border/50 grid grid-cols-3 gap-2 text-caption text-muted-foreground font-mono">
              <div>Elapsed: <span class="text-foreground">{duration(session.elapsed_seconds)}</span></div>
              <div>CPU: <span class="text-foreground">{session.cpu_percent?.toFixed(1) ?? '0.0'}%</span></div>
              <div>Memory: <span class="text-foreground">{formatBytes(session.memory_bytes)}</span></div>
            </div>
          </Card>
        {/each}
      </div>
    {/if}
  </div>

  <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
    <Card class="p-4 bg-card/70 border-border/70 space-y-3">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2">
          <Server size={16} class="text-primary" />
          <h4 class="text-xs font-semibold uppercase tracking-wider">Development Services</h4>
        </div>
        {#if onNavigateTab}
          <Button
            variant="ghost"
            size="sm"
            class="text-caption gap-1 text-muted-foreground hover:text-foreground"
            onclick={() => onNavigateTab?.('development_servers')}
          >
            View Dev Servers <ExternalLink size={11} />
          </Button>
        {/if}
      </div>

      {#if project.dev_ports.length === 0}
        <p class="text-caption text-muted-foreground py-2">No active development listeners detected for this project.</p>
      {:else}
        <div class="flex flex-wrap gap-2 pt-1">
          {#each project.dev_ports as port}
            <div class="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-secondary/80 text-foreground text-xs font-mono">
              <span class="w-1.5 h-1.5 rounded-full bg-success"></span>
              <span>localhost:{port}</span>
            </div>
          {/each}
        </div>
      {/if}
    </Card>

    <Card class="p-4 bg-card/70 border-border/70 space-y-3">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2">
          <HardDrive size={16} class="text-primary" />
          <h4 class="text-xs font-semibold uppercase tracking-wider">Developer Storage</h4>
        </div>
        {#if onNavigateTab}
          <Button
            variant="ghost"
            size="sm"
            class="text-caption gap-1 text-muted-foreground hover:text-foreground"
            onclick={() => onNavigateTab?.('developer-artifacts')}
          >
            View Artifacts <ExternalLink size={11} />
          </Button>
        {/if}
      </div>

      {#if project.artifact_size_bytes}
        <div class="pt-1">
          <div class="text-lg font-semibold tabular-nums">{formatBytes(project.artifact_size_bytes)}</div>
          <p class="text-caption text-muted-foreground mt-0.5">
            Correlated node_modules, build caches, and test artifacts in this project root.
          </p>
        </div>
      {:else}
        <p class="text-caption text-muted-foreground py-2">No developer artifact total recorded for this root.</p>
      {/if}
    </Card>
  </div>
</div>
