<script lang="ts">
  import { onMount } from 'svelte';
  import {
    Activity,
    ArrowLeft,
    Bot,
    ExternalLink,
    FolderGit2,
    FolderOpen,
    HardDrive,
    RefreshCw,
    Server,
    Square,
    Terminal,
  } from 'lucide-svelte';
  import type {
    AgentActivityStatus,
    AgentAdapterHealth,
    AgentAdapterState,
    AgentEvidence,
    AgentSession,
    AttentionReason,
    ProjectContext,
  } from '../../lib/models/types';
  import { agentActivityStore } from '../../lib/stores/agentActivity.svelte';
  import { usageStore } from '../../lib/stores/usage.svelte';
  import { formatBytes } from '../../lib/utils/format';
  import { tauriRevealInFinder } from '../../lib/utils/tauri';
  import Badge from '../../lib/components/Badge.svelte';
  import AiUsageCards from '../../lib/components/AiUsageCards.svelte';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';

  interface Props {
    onNavigateTab?: (tab: string) => void;
  }

  let { onNavigateTab }: Props = $props();

  let snapshot = $derived(agentActivityStore.snapshot);
  let integrations = $derived(agentActivityStore.integrations);
  let selectedProject = $derived(agentActivityStore.selectedProject);
  let usageSnapshot = $derived(usageStore.snapshot);
  let stoppingSessionIds = $state<Set<string>>(new Set());
  let actionFeedback = $state<string | null>(null);

  onMount(() => {
    void agentActivityStore.refresh();
    void agentActivityStore.fetchIntegrations();
    void usageStore.refreshIfStale();
  });

  async function handleRefreshAll() {
    await Promise.all([
      agentActivityStore.refresh(true),
      usageStore.refresh(true),
    ]);
  }

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

  function adapterStatusBadge(stateVal: AgentAdapterState) {
    switch (stateVal) {
      case 'connected':
        return { label: 'Connected', variant: 'success' as const };
      case 'integration_available':
        return { label: 'Integration Available', variant: 'warning' as const };
      case 'process_only':
        return { label: 'Process Only', variant: 'secondary' as const };
      case 'not_installed':
        return { label: 'Not Installed', variant: 'outline' as const };
      case 'version_unsupported':
        return { label: 'Version Unsupported', variant: 'danger' as const };
      case 'partial':
        return { label: 'Partial', variant: 'warning' as const };
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

  async function handleUninstallIntegration(toolId: string) {
    try {
      const res = await agentActivityStore.uninstallIntegration(toolId);
      actionFeedback = res.message;
    } catch (err) {
      actionFeedback = `Uninstall failed: ${err instanceof Error ? err.message : String(err)}`;
    }
  }

  function handleRevealInFinder(path: string) {
    void tauriRevealInFinder(path);
  }

  function handleOpenInTerminal(path: string) {
    void agentActivityStore.openInTerminal(path);
  }
</script>

<div class="space-y-6 max-w-5xl">
  {#if selectedProject}
    <!-- ================= LEVEL 2: PROJECT COCKPIT ================= -->
    <div class="space-y-6">
      <div class="flex items-center justify-between gap-4">
        <Button
          variant="ghost"
          size="sm"
          onclick={() => agentActivityStore.selectProject(null)}
          class="gap-1.5 -ml-2 text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft size={14} />
          Back to Projects
        </Button>
        <div class="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onclick={() => handleRevealInFinder(selectedProject.identity.display_path)}
            title="Reveal repository folder in Finder"
          >
            <FolderOpen size={13} />
            Reveal in Finder
          </Button>
          <Button
            variant="outline"
            size="sm"
            onclick={() => handleOpenInTerminal(selectedProject.identity.display_path)}
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
          >
            <RefreshCw size={13} class={agentActivityStore.isLoading ? 'animate-spin' : ''} />
            Refresh
          </Button>
        </div>
      </div>

      <!-- Project Header Card -->
      <Card class="p-5 bg-card/70 border-border/70">
        <div class="flex flex-col md:flex-row md:items-center justify-between gap-4">
          <div>
            <div class="flex items-center gap-2.5 flex-wrap">
              <h2 class="text-xl font-bold tracking-tight">{selectedProject.identity.display_name}</h2>
              {#if selectedProject.identity.is_worktree}
                <Badge variant="outline">Worktree</Badge>
              {/if}
              {#if selectedProject.identity.branch}
                <Badge variant="secondary" class="font-mono">{selectedProject.identity.branch}</Badge>
              {/if}
              {#if selectedProject.identity.is_detached}
                <Badge variant="warning">Detached HEAD</Badge>
              {/if}
              {#if selectedProject.identity.is_dirty}
                <Badge variant="warning">Modified working tree</Badge>
              {/if}
            </div>
            <p class="mt-1 font-mono text-xs text-muted-foreground">
              {selectedProject.identity.display_path}
            </p>
          </div>

          <div class="flex items-center gap-4 text-xs text-muted-foreground divide-x divide-border/60">
            <div>
              <span class="font-semibold text-foreground">{selectedProject.sessions.length}</span>
              <span> agent sessions</span>
            </div>
            {#if selectedProject.dev_ports.length > 0}
              <div class="pl-4">
                <span class="font-semibold text-foreground">{selectedProject.dev_ports.length}</span>
                <span> dev ports</span>
              </div>
            {/if}
            {#if selectedProject.artifact_size_bytes}
              <div class="pl-4">
                <span class="font-semibold text-foreground">{formatBytes(selectedProject.artifact_size_bytes)}</span>
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

      <!-- Agent Sessions Section -->
      <div class="space-y-3">
        <div class="flex items-center justify-between">
          <h3 class="text-sm font-semibold tracking-tight">Active Agent Sessions</h3>
          <span class="text-caption text-muted-foreground">Graceful termination sends SIGTERM only</span>
        </div>

        {#if selectedProject.sessions.length === 0}
          <Card class="p-8 text-center bg-card/50">
            <Bot size={24} class="mx-auto text-muted-foreground mb-2" />
            <p class="text-xs text-muted-foreground">No active agent sessions running in this project.</p>
          </Card>
        {:else}
          <div class="space-y-3">
            {#each selectedProject.sessions as session (session.id)}
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

      <!-- Correlated Dev Services & Storage -->
      <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
        <!-- Development Services -->
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
                onclick={() => onNavigateTab('development_servers')}
              >
                View Dev Servers <ExternalLink size={11} />
              </Button>
            {/if}
          </div>

          {#if selectedProject.dev_ports.length === 0}
            <p class="text-caption text-muted-foreground py-2">No active development listeners detected for this project.</p>
          {:else}
            <div class="flex flex-wrap gap-2 pt-1">
              {#each selectedProject.dev_ports as port}
                <div class="flex items-center gap-1.5 px-2.5 py-1 rounded-md bg-secondary/80 text-foreground text-xs font-mono">
                  <span class="w-1.5 h-1.5 rounded-full bg-success"></span>
                  <span>localhost:{port}</span>
                </div>
              {/each}
            </div>
          {/if}
        </Card>

        <!-- Correlated Storage -->
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
                onclick={() => onNavigateTab('developer-artifacts')}
              >
                View Artifacts <ExternalLink size={11} />
              </Button>
            {/if}
          </div>

          {#if selectedProject.artifact_size_bytes}
            <div class="pt-1">
              <div class="text-lg font-semibold tabular-nums">
                {formatBytes(selectedProject.artifact_size_bytes)}
              </div>
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
  {:else}
    <!-- ================= LEVEL 1: PROJECT LIST ================= -->
    <header class="flex items-start justify-between gap-4 border-b border-border/60 pb-4">
      <div class="flex items-center gap-3 min-w-0">
        <div class="h-9 w-9 shrink-0 rounded-lg bg-secondary text-foreground flex items-center justify-center">
          <FolderGit2 size={19} />
        </div>
        <div class="min-w-0">
          <div class="flex items-center gap-2">
            <h2 class="text-base font-semibold tracking-tight">AI Activity & Projects</h2>
            <Badge variant="outline">Local only</Badge>
          </div>
          <p class="mt-0.5 text-xs text-muted-foreground">
            Connected AI account limits, active agent sessions, dev listeners, and local workspace storage.
          </p>
        </div>
      </div>
      <Button
        variant="outline"
        size="sm"
        disabled={agentActivityStore.isLoading || usageStore.isLoading}
        ariaLabel="Refresh AI activity"
        title="Refresh AI activity"
        onclick={handleRefreshAll}
      >
        <RefreshCw size={13} class={agentActivityStore.isLoading || usageStore.isLoading ? 'animate-spin' : ''} />
        {agentActivityStore.isLoading || usageStore.isLoading ? 'Refreshing' : 'Refresh'}
      </Button>
    </header>

    {#if agentActivityStore.error}
      <div role="alert" class="rounded-xl border border-destructive/20 bg-destructive/5 px-4 py-3 text-xs text-destructive">
        Refresh failed. The last successful local snapshot remains visible. {agentActivityStore.error}
      </div>
    {/if}

    {#if actionFeedback}
      <div role="status" class="rounded-xl border border-primary/20 bg-primary/5 px-4 py-3 text-xs text-primary">
        {actionFeedback}
      </div>
    {/if}

    {#if !snapshot && agentActivityStore.isLoading}
      <div aria-label="Loading project activity" class="space-y-3">
        {#each Array(3) as _}
          <div class="h-28 animate-pulse rounded-xl border border-border/60 bg-secondary/30"></div>
        {/each}
      </div>
    {:else if snapshot}
      <!-- Connected AI Accounts & Quota -->
      <section aria-label="Connected AI Accounts" class="space-y-3">
        <div class="flex items-center justify-between gap-3">
          <div>
            <h3 class="text-sm font-semibold">AI Accounts & Quota</h3>
            <p class="text-caption text-muted-foreground">
              Official-client usage metadata and local coding-agent activity. OAuth token files never reach the UI.
            </p>
          </div>
          {#if usageSnapshot}
            <span class="shrink-0 text-caption text-muted-foreground">
              {usageSnapshot.providers.filter((provider) => provider.connected).length} connected
            </span>
          {/if}
        </div>
        {#if usageStore.error}
          <div role="alert" class="rounded-xl border border-destructive/20 bg-destructive/5 px-4 py-3 text-xs text-destructive">
            {usageStore.error}
          </div>
        {:else if usageStore.isLoading && !usageSnapshot}
          <div class="grid grid-cols-1 gap-3 md:grid-cols-2" aria-label="Loading AI account usage">
            <div class="h-[190px] animate-pulse rounded-xl border border-border/60 bg-secondary/30"></div>
            <div class="h-[190px] animate-pulse rounded-xl border border-border/60 bg-secondary/30"></div>
          </div>
        {:else if usageSnapshot && usageSnapshot.providers.length > 0}
          <AiUsageCards
            providers={usageSnapshot.providers}
            connectingProvider={usageStore.connectingProvider}
            onConnectOpenRouter={() => usageStore.connectOpenRouter()}
          />
        {:else}
          <Card class="p-4 bg-card/60 text-xs text-muted-foreground">
            No AI account usage metadata is available yet.
          </Card>
        {/if}
      </section>

      <!-- Metric Summary Cards -->
      <section aria-label="Project activity summary" class="grid grid-cols-4 gap-3">
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
        <!-- Project Cards List -->
        <section aria-label="Canonical projects" class="space-y-4">
          {#each snapshot.projects as project (project.identity.id)}
            {@const hasAttention = project.sessions.some((s) => s.attention_reason != null)}
            <Card
              class="p-5 bg-card/70 hover:border-primary/40 transition-colors cursor-pointer border-border/70"
              onclick={() => agentActivityStore.selectProject(project.identity.id)}
            >
              <div class="flex items-start justify-between gap-4">
                <div class="min-w-0">
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
                </div>

                <div class="flex items-center gap-2 shrink-0">
                  <Badge variant={project.sessions.length > 0 ? 'success' : 'secondary'}>
                    {project.sessions.length} {project.sessions.length === 1 ? 'agent' : 'agents'}
                  </Badge>
                </div>
              </div>

              <!-- Correlated quick chips -->
              <div class="mt-3 flex items-center gap-2 flex-wrap text-caption text-muted-foreground">
                {#if project.dev_ports.length > 0}
                  <span class="inline-flex items-center gap-1 bg-secondary/80 px-2 py-0.5 rounded font-mono text-foreground">
                    <Server size={10} />
                    {project.dev_ports.map((p) => `:${p}`).join(', ')}
                  </span>
                {/if}
                {#if project.artifact_size_bytes}
                  <span class="inline-flex items-center gap-1 bg-secondary/80 px-2 py-0.5 rounded font-mono text-foreground">
                    <HardDrive size={10} />
                    {formatBytes(project.artifact_size_bytes)}
                  </span>
                {/if}
              </div>

              <!-- Sessions Preview -->
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

      <!-- Unassigned Sessions Section -->
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
                    Stop
                  </Button>
                {/if}
              </div>
            {/each}
          </Card>
        </section>
      {/if}

      <!-- Supported Adapters & Integration Hub Section -->
      <section aria-label="Supported tool adapters" class="space-y-3 pt-4 border-t border-border/60">
        <div class="flex items-center justify-between">
          <div>
            <h3 class="text-sm font-semibold">Tool Adapters</h3>
            <p class="text-caption text-muted-foreground">
              Exact local process observation across supported AI developer tools.
            </p>
          </div>
        </div>

        <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
          {#each snapshot.adapters as adapter (adapter.tool_id)}
            {@const status = adapterStatusBadge(adapter.state)}
            {@const integration = integrations.find((i) => i.tool_id === adapter.tool_id)}
            <Card class="p-4 bg-card/60 border-border/70 space-y-2.5">
              <div class="flex items-start justify-between gap-2">
                <div>
                  <div class="flex items-center gap-2">
                    <span class="text-xs font-bold">{adapter.display_name}</span>
                    <Badge variant={status.variant}>{status.label}</Badge>
                  </div>
                  <p class="text-caption text-muted-foreground mt-1">{adapter.message}</p>
                </div>

                {#if integration?.integration_active}
                  <div>
                    <Button
                      variant="outline"
                      size="sm"
                      class="text-caption text-destructive hover:bg-destructive/10"
                      onclick={() => handleUninstallIntegration(adapter.tool_id)}
                    >
                      Remove legacy marker
                    </Button>
                  </div>
                {/if}
              </div>

              {#if integration?.integration_active && integration.config_path}
                <div class="font-mono text-caption text-muted-foreground truncate pt-1 border-t border-border/40">
                  Config: {integration.config_path}
                </div>
              {/if}
            </Card>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</div>
