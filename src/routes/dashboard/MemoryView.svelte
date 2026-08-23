<script lang="ts">
  import { onMount, tick } from 'svelte';
  import type { DevelopmentListener, ProcessMemory } from '../../lib/models/types';
  import { memoryStore } from '../../lib/stores/memory.svelte';
  import {
    developmentPortsStore,
    filterDevelopmentListeners,
  } from '../../lib/stores/developmentPorts.svelte';
  import { formatBytes, formatProcessAge } from '../../lib/utils/format';
  import { withMinimumDuration } from '../../lib/utils/async';
  import { filterProcesses } from '../../lib/utils/memory';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import ProgressBar from '../../lib/components/ProgressBar.svelte';
  import {
    Activity,
    RotateCw,
    Layers,
    Cpu,
    Database,
    LogOut,
    TriangleAlert,
    Search,
    X,
    Server,
    Globe,
    Radio,
    ShieldAlert,
    ShieldCheck,
  } from 'lucide-svelte';

  onMount(() => {
    function updatePolling() {
      if (typeof document !== 'undefined' && document.visibilityState === 'visible') {
        memoryStore.startPolling(2500);
        developmentPortsStore.startPolling(15000);
      } else {
        memoryStore.stopPolling();
        developmentPortsStore.stopPolling();
      }
    }

    updatePolling();
    document.addEventListener('visibilitychange', updatePolling);

    return () => {
      document.removeEventListener('visibilitychange', updatePolling);
      memoryStore.stopPolling();
      developmentPortsStore.stopPolling();
    };
  });

  let memory = $derived(memoryStore.memory);
  let pendingProcess = $state<ProcessMemory | null>(null);
  let isRefreshing = $state(false);
  let searchQuery = $state('');

  // Development Ports State
  let devPortSearch = $state('');
  let isRefreshingPorts = $state(false);
  let pendingReleaseListener = $state<DevelopmentListener | null>(null);
  let pendingForceListener = $state<DevelopmentListener | null>(null);
  let releaseReturnFocusId = $state<string | null>(null);

  let filteredProcesses = $derived(
    filterProcesses(memory?.top_processes ?? [], searchQuery)
  );

  let filteredDevPorts = $derived(
    filterDevelopmentListeners(developmentPortsStore.listeners, devPortSearch)
  );

  let topReclaimableApp = $derived(
    memory?.top_processes.find((p) => p.can_terminate && p.memory_bytes > 400 * 1024 * 1024)
  );

  let memoryHealthTitle = $derived.by(() => {
    if (!memory) return 'Reading memory metrics…';
    if (memory.pressure === 'critical') return 'Memory pressure is high';
    if (memory.pressure === 'warning') return 'Elevated memory usage';
    return 'Memory looks healthy';
  });

  let memoryHealthSubtitle = $derived.by(() => {
    if (!memory) return '';
    if (memory.pressure === 'critical' || memory.pressure === 'warning') {
      if (topReclaimableApp) {
        return `Closing ${topReclaimableApp.name} could recover ~${formatBytes(topReclaimableApp.memory_bytes)}.`;
      }
      return 'Consider closing background developer apps to relieve system memory.';
    }
    return 'System memory allocations and swap are well within safe thresholds. No action needed.';
  });

  async function handleRefresh() {
    if (isRefreshing) return;
    isRefreshing = true;
    try {
      await withMinimumDuration(memoryStore.refreshMemory(), 600);
    } finally {
      isRefreshing = false;
    }
  }

  async function handleRefreshPorts() {
    if (isRefreshingPorts) return;
    isRefreshingPorts = true;
    try {
      await withMinimumDuration(developmentPortsStore.refresh(), 500);
    } finally {
      isRefreshingPorts = false;
    }
  }

  async function terminatePending(force: boolean) {
    if (!pendingProcess) return;
    const name = pendingProcess.name;
    pendingProcess = null;
    await memoryStore.terminateProcessGroup(name, force);
  }

  async function handleReleaseNormally() {
    if (!pendingReleaseListener) return;
    const listener = pendingReleaseListener;
    pendingReleaseListener = null;
    await restoreReleaseFocus();

    try {
      const result = await developmentPortsStore.release(listener, 'graceful');
      if (result.outcome === 'still_listening' && result.listener) {
        // Show the secondary force confirmation dialog
        pendingForceListener = result.listener;
        await focusPortDialog('force-release-cancel');
      }
    } catch {
      // Error is set in store
    }
  }

  async function handleForceRelease() {
    if (!pendingForceListener) return;
    const listener = pendingForceListener;
    pendingForceListener = null;
    await restoreReleaseFocus();

    try {
      await developmentPortsStore.release(listener, 'force');
    } catch {
      // Error is set in store
    }
  }

  function releaseButtonId(listener: DevelopmentListener) {
    const address = listener.bind_address.replace(/[^a-zA-Z0-9]/g, '-');
    return `release-port-${listener.pid}-${listener.port}-${address}`;
  }

  function openReleaseDialog(listener: DevelopmentListener) {
    releaseReturnFocusId = releaseButtonId(listener);
    pendingReleaseListener = listener;
    pendingForceListener = null;
    void focusPortDialog('release-cancel');
  }

  async function focusPortDialog(id: string) {
    await tick();
    document.getElementById(id)?.focus();
  }

  async function restoreReleaseFocus() {
    await tick();
    if (releaseReturnFocusId) document.getElementById(releaseReturnFocusId)?.focus();
  }

  function closePortDialogs() {
    pendingReleaseListener = null;
    pendingForceListener = null;
    void restoreReleaseFocus();
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      if (pendingProcess) pendingProcess = null;
      if (pendingReleaseListener || pendingForceListener) closePortDialogs();
    }
  }

  const pressureColors = {
    normal: 'text-success bg-success/10 border-success/20',
    warning: 'text-warning bg-warning/10 border-warning/20',
    critical: 'text-destructive bg-destructive/10 border-destructive/20',
  };
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="space-y-6">
  <!-- Header -->
  <div class="flex items-center justify-between pb-3 border-b border-border/60">
    <div class="flex items-center gap-3">
      <div class="h-9 w-9 rounded-lg bg-success/10 text-success flex items-center justify-center">
        <Activity size={20} />
      </div>
      <div>
        <div class="flex items-center gap-2">
          <h2 class="text-base font-semibold text-foreground tracking-tight">{memoryHealthTitle}</h2>
          {#if memory}
            <div class="px-2.5 py-0.5 rounded-full text-caption font-mono font-medium border flex items-center gap-1.5 {pressureColors[memory.pressure]}">
              <span class="h-1.5 w-1.5 rounded-full {memory.pressure === 'critical' ? 'bg-destructive animate-pulse-soft' : memory.pressure === 'warning' ? 'bg-warning' : 'bg-success'}"></span>
              <span>Pressure: {memory.pressure.toUpperCase()}</span>
            </div>
          {/if}
        </div>
        <p class="text-xs text-muted-foreground mt-0.5">
          {memoryHealthSubtitle}
        </p>
      </div>
    </div>

    <Button
      variant="outline"
      size="sm"
      disabled={isRefreshing || memoryStore.isLoading}
      onclick={handleRefresh}
      class="gap-1.5 text-xs"
    >
      <RotateCw size={13} class={isRefreshing || memoryStore.isLoading ? 'animate-gentle-spin' : ''} />
      <span>Refresh</span>
    </Button>
  </div>

  {#if memoryStore.error}
    <div class="rounded-xl border border-destructive/20 bg-destructive/5 px-4 py-3 text-xs text-destructive">{memoryStore.error}</div>
  {:else if memoryStore.lastAction}
    <div class="rounded-xl border border-success/20 bg-success/5 px-4 py-3 text-xs text-success">{memoryStore.lastAction} macOS may retain some memory as reusable cache.</div>
  {/if}

  {#if memory}
    <!-- Key Memory Gauges -->
    <div class="grid grid-cols-1 sm:grid-cols-3 gap-3">
      <!-- Total / Used RAM -->
      <Card class="p-4 space-y-2 bg-card/60">
        <div class="flex items-center justify-between text-xs text-muted-foreground font-medium">
          <span>Physical Memory</span>
          <Cpu size={15} />
        </div>
        <div class="text-2xl font-bold font-mono text-foreground">
          {formatBytes(memory.used_bytes)} / {formatBytes(memory.total_bytes)}
        </div>
        <ProgressBar
          value={(memory.used_bytes / memory.total_bytes) * 100}
          height="h-2"
          color={memory.pressure === 'critical' ? 'bg-destructive' : memory.pressure === 'warning' ? 'bg-warning' : 'bg-success'}
        />
        <div class="flex justify-between text-caption text-muted-foreground font-mono">
          <span>Available: {formatBytes(memory.available_bytes)}</span>
          <span>Free: {formatBytes(memory.free_bytes)}</span>
        </div>
      </Card>

      <!-- Compressed Memory -->
      <Card class="p-4 space-y-2 bg-card/60">
        <div class="flex items-center justify-between text-xs text-muted-foreground font-medium">
          <span>Compressed Memory</span>
          <Layers size={15} class="text-purple-400" />
        </div>
        <div class="text-2xl font-bold font-mono text-foreground">
          {formatBytes(memory.compressed_bytes)}
        </div>
        <p class="text-meta text-muted-foreground mt-1">
          macOS in-RAM memory compression avoiding disk swap slowdowns.
        </p>
      </Card>

      <!-- Swap Usage -->
      <Card class="p-4 space-y-2 bg-card/60">
        <div class="flex items-center justify-between text-xs text-muted-foreground font-medium">
          <span>Swap Space Used</span>
          <Database size={15} class="text-blue-400" />
        </div>
        <div class="text-2xl font-bold font-mono text-foreground">
          {formatBytes(memory.swap_used_bytes)}
          {#if memory.swap_total_bytes > 0}
            <span class="text-xs font-normal text-muted-foreground">/ {formatBytes(memory.swap_total_bytes)}</span>
          {/if}
        </div>
        {#if memory.swap_total_bytes > 0}
          <ProgressBar
            value={(memory.swap_used_bytes / memory.swap_total_bytes) * 100}
            height="h-1.5"
            color="bg-blue-500"
          />
        {/if}
        <p class="text-meta text-muted-foreground mt-1">
          Secondary disk paging memory usage.
        </p>
      </Card>
    </div>

    <!-- Top Developer Processes Table -->
    <div class="space-y-3 pt-2">
      <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
        <div class="flex items-center gap-2">
          <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
            Top Resource Consuming Processes
          </h3>
          <span class="text-caption text-muted-foreground font-mono bg-secondary/80 px-1.5 py-0.5 rounded">
            2.5s live
          </span>
        </div>

        <div class="relative w-full sm:w-64">
          <Search size={14} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
          <input
            type="text"
            bind:value={searchQuery}
            placeholder="Search process or PID…"
            aria-label="Search processes by name or PID"
            class="w-full h-8 pl-8 pr-7 text-xs rounded-lg border border-border bg-card text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
          />
          {#if searchQuery}
            <button
              type="button"
              onclick={() => (searchQuery = '')}
              class="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              aria-label="Clear search"
            >
              <X size={13} />
            </button>
          {/if}
        </div>
      </div>

      {#if filteredProcesses.length > 0}
        <div class="border border-border/80 rounded-xl overflow-hidden bg-card/70 divide-y divide-border/60">
          {#each filteredProcesses as proc (proc.name)}
            <div class="group flex items-center justify-between p-3 text-xs hover:bg-secondary/30 transition-colors">
              <div class="flex items-center gap-3 min-w-0 pr-2">
                <div
                  class="font-mono text-meta text-muted-foreground w-12 shrink-0"
                  title={proc.pids && proc.pids.length > 1 ? `PIDs: ${proc.pids.join(', ')}` : undefined}
                >
                  PID {proc.pid}
                </div>
                <div class="min-w-0 truncate">
                  <span class="font-medium text-foreground">{proc.name}</span>
                  {#if proc.process_count > 1}
                    <span class="text-muted-foreground ml-1.5 text-meta">
                      ({proc.process_count} instances)
                    </span>
                  {/if}
                </div>
              </div>

              <div class="flex items-center gap-4 shrink-0">
                <span class="font-mono font-semibold text-foreground">
                  {formatBytes(proc.memory_bytes)}
                </span>
                {#if proc.can_terminate}
                  <Button
                    variant="outline"
                    size="sm"
                    class="gap-1.5 opacity-70 group-hover:opacity-100"
                    disabled={memoryStore.terminating !== null}
                    onclick={() => (pendingProcess = proc)}
                  >
                    <LogOut size={12} />
                    Quit
                  </Button>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      {:else if searchQuery.trim()}
        <div class="p-8 text-center border border-border/80 rounded-xl bg-card/70 space-y-2">
          <p class="text-xs text-muted-foreground">No processes matching "{searchQuery}"</p>
          <Button variant="ghost" size="sm" onclick={() => (searchQuery = '')} class="text-xs">
            Clear Search
          </Button>
        </div>
      {:else}
        <div class="p-8 text-center border border-border/80 rounded-xl bg-card/70 text-xs text-muted-foreground">
          No high-memory processes detected.
        </div>
      {/if}
    </div>

    <!-- Development Servers Section -->
    <div class="space-y-3 pt-4 border-t border-border/60">
      <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
        <div class="flex items-center gap-2">
          <div class="h-6 w-6 rounded-md bg-blue-500/10 text-blue-400 flex items-center justify-center">
            <Server size={14} />
          </div>
          <div>
            <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider">
              Development Servers
            </h3>
          </div>
          <span class="text-caption text-muted-foreground font-mono bg-secondary/80 px-1.5 py-0.5 rounded">
            TCP Listeners
          </span>
        </div>

        <div class="flex items-center gap-2">
          <div class="relative w-full sm:w-56">
            <Search size={14} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
            <input
              type="text"
              bind:value={devPortSearch}
              placeholder="Search port, server, project…"
              aria-label="Search development server ports"
              class="w-full h-8 pl-8 pr-7 text-xs rounded-lg border border-border bg-card text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
            />
            {#if devPortSearch}
              <button
                type="button"
                onclick={() => (devPortSearch = '')}
                class="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                aria-label="Clear port search"
              >
                <X size={13} />
              </button>
            {/if}
          </div>

          <Button
            variant="outline"
            size="sm"
            disabled={isRefreshingPorts || developmentPortsStore.isLoading}
            onclick={handleRefreshPorts}
            class="gap-1.5 text-xs shrink-0"
            title="Refresh development server listeners"
          >
            <RotateCw size={12} class={isRefreshingPorts || developmentPortsStore.isLoading ? 'animate-gentle-spin' : ''} />
            <span>Refresh</span>
          </Button>
        </div>
      </div>

      {#if developmentPortsStore.error}
        <div class="rounded-xl border border-destructive/20 bg-destructive/5 px-4 py-2.5 text-xs text-destructive flex items-center justify-between">
          <span>{developmentPortsStore.error}</span>
          <button
            type="button"
            onclick={() => developmentPortsStore.clearError()}
            class="text-destructive/70 hover:text-destructive text-meta ml-2"
            aria-label="Dismiss error"
          >
            Dismiss
          </button>
        </div>
      {:else if developmentPortsStore.lastAction}
        <div class="rounded-xl border border-success/20 bg-success/5 px-4 py-2.5 text-xs text-success flex items-center justify-between">
          <span>{developmentPortsStore.lastAction}</span>
          <button
            type="button"
            onclick={() => developmentPortsStore.clearLastAction()}
            class="text-success/70 hover:text-success text-meta ml-2"
            aria-label="Dismiss message"
          >
            Dismiss
          </button>
        </div>
      {/if}

      {#if filteredDevPorts.length > 0}
        <div class="border border-border/80 rounded-xl overflow-hidden bg-card/70 divide-y divide-border/60">
          {#each filteredDevPorts as portItem (portItem.id)}
            <div class="group flex flex-col sm:flex-row sm:items-center justify-between p-3 gap-2.5 text-xs hover:bg-secondary/30 transition-colors">
              <div class="flex items-center gap-3 min-w-0">
                <!-- Port & Protocol Badge -->
                <div class="w-20 shrink-0 font-mono font-bold text-sm text-foreground">
                  {portItem.port}<span class="text-caption font-normal text-muted-foreground ml-0.5">/TCP</span>
                </div>

                <!-- Server Label & Project Context -->
                <div class="min-w-0 space-y-0.5">
                  <div class="flex items-center gap-2 flex-wrap">
                    <span class="font-semibold text-foreground">{portItem.server_name}</span>
                    {#if portItem.project_name}
                      <span class="text-muted-foreground text-caption font-medium bg-secondary px-1.5 py-0.5 rounded">
                        {portItem.project_name}
                      </span>
                    {/if}
                    <span class="font-mono text-meta text-muted-foreground">
                      PID {portItem.pid}
                    </span>
                  </div>
                  {#if portItem.working_directory}
                    <p class="text-meta text-muted-foreground truncate font-mono" title={portItem.working_directory}>
                      {portItem.working_directory}
                    </p>
                  {/if}
                </div>
              </div>

              <div class="flex items-center justify-between sm:justify-end gap-3 shrink-0 pt-1 sm:pt-0">
                <!-- Bind Address & Exposure -->
                <div class="flex items-center gap-1.5 font-mono text-meta">
                  <span class="text-muted-foreground">{portItem.bind_address}</span>
                  {#if portItem.exposure === 'all_interfaces'}
                    <span class="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-meta font-medium bg-warning/10 text-warning border border-warning/20" title="Exposed to all local and external network interfaces">
                      <TriangleAlert size={10} />
                      All interfaces
                    </span>
                  {:else if portItem.exposure === 'network'}
                    <span class="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-meta font-medium bg-blue-500/10 text-blue-400 border border-blue-500/20">
                      <Globe size={10} />
                      Network
                    </span>
                  {:else}
                    <span class="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-meta font-medium bg-secondary text-muted-foreground">
                      Loopback
                    </span>
                  {/if}
                </div>

                <!-- Process Age -->
                <span class="font-mono text-meta text-muted-foreground w-14 text-right shrink-0">
                  {formatProcessAge(portItem.started_at)}
                </span>

                <!-- Action Button or Blocked State -->
                <div class="w-20 text-right shrink-0">
                  {#if portItem.can_release}
                    <Button
                      id={releaseButtonId(portItem)}
                      variant="outline"
                      size="sm"
                      class="gap-1 text-xs opacity-80 group-hover:opacity-100 hover:border-destructive/40 hover:text-destructive"
                      disabled={developmentPortsStore.releasingId !== null}
                      onclick={() => openReleaseDialog(portItem)}
                      title="Request graceful release of port {portItem.port}"
                    >
                      <LogOut size={12} />
                      Release
                    </Button>
                  {:else}
                    <span
                      class="inline-flex items-center gap-1 text-meta font-medium text-muted-foreground/70 bg-secondary/40 px-2 py-1 rounded cursor-help"
                      title={portItem.blocked_reason || 'Protected process cannot be released'}
                    >
                      <ShieldCheck size={11} class="opacity-70" />
                      Protected
                    </span>
                  {/if}
                </div>
              </div>
            </div>
          {/each}
        </div>
      {:else if devPortSearch.trim()}
        <div class="p-8 text-center border border-border/80 rounded-xl bg-card/70 space-y-2">
          <p class="text-xs text-muted-foreground">No development servers matching "{devPortSearch}"</p>
          <Button variant="ghost" size="sm" onclick={() => (devPortSearch = '')} class="text-xs">
            Clear Search
          </Button>
        </div>
      {:else}
        <div class="p-8 text-center border border-border/80 rounded-xl bg-card/70 text-xs text-muted-foreground space-y-1">
          <p class="font-medium text-foreground">No user-owned development servers are listening.</p>
          <p class="text-meta text-muted-foreground">When you start tools like Vite, Next.js, or Astro, active listening ports will appear here.</p>
        </div>
      {/if}
    </div>
  {:else}
    <div class="py-16 text-center text-xs text-muted-foreground space-y-2">
      <RotateCw size={20} class="animate-gentle-spin mx-auto opacity-50" />
      <p>Reading macOS memory statistics...</p>
    </div>
  {/if}

  <!-- Quit Process Group Modal -->
  {#if pendingProcess}
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-4 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="terminate-title">
      <Card class="w-full max-w-md space-y-4 border-border bg-card p-5 shadow-2xl">
        <div class="flex items-start gap-3">
          <div class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-warning/10 text-warning">
            <TriangleAlert size={17} />
          </div>
          <div>
            <h3 id="terminate-title" class="text-sm font-semibold">Quit {pendingProcess.name}?</h3>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
              This group contains {pendingProcess.process_count} processes using approximately {formatBytes(pendingProcess.memory_bytes)}. Unsaved work, active downloads, or running tasks may be lost.
            </p>
          </div>
        </div>

        <div class="rounded-lg border border-border/70 bg-secondary/40 px-3 py-2.5 text-meta leading-relaxed text-muted-foreground">
          Try normal Quit first. Force Quit stops every matching process immediately and should only be used when the app does not respond.
        </div>

        <div class="flex justify-end gap-2 pt-1">
          <Button variant="ghost" size="sm" onclick={() => (pendingProcess = null)}>Cancel</Button>
          <Button variant="outline" size="sm" onclick={() => terminatePending(false)}>Quit Normally</Button>
          <Button variant="destructive" size="sm" onclick={() => terminatePending(true)}>Force Quit</Button>
        </div>
      </Card>
    </div>
  {/if}

  <!-- Graceful Release Port Modal -->
  {#if pendingReleaseListener}
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-4 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="release-title">
      <Card class="w-full max-w-md space-y-4 border-border bg-card p-5 shadow-2xl">
        <div class="flex items-start gap-3">
          <div class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-warning/10 text-warning">
            <Radio size={17} />
          </div>
          <div>
            <h3 id="release-title" class="text-sm font-semibold">
              Release {pendingReleaseListener.server_name} on {pendingReleaseListener.bind_address}:{pendingReleaseListener.port}?
            </h3>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
              This will request graceful termination (SIGTERM) for PID {pendingReleaseListener.pid}
              {#if pendingReleaseListener.project_name}
                belonging to project <span class="font-semibold text-foreground">{pendingReleaseListener.project_name}</span>
              {/if}
              .
            </p>
          </div>
        </div>

        <div class="rounded-lg border border-border/70 bg-secondary/40 px-3 py-2.5 text-meta leading-relaxed text-muted-foreground">
          Active browser tabs, hot module reload sessions, or in-flight HTTP requests to this server will stop. Zenith will check if the port is freed.
        </div>

        <div class="flex justify-end gap-2 pt-1">
          <Button id="release-cancel" variant="ghost" size="sm" onclick={closePortDialogs}>
            Cancel
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={developmentPortsStore.releasingId !== null}
            onclick={handleReleaseNormally}
          >
            Release Normally
          </Button>
        </div>
      </Card>
    </div>
  {/if}

  <!-- Force Release Port Modal (Triggered when process ignored SIGTERM) -->
  {#if pendingForceListener}
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-4 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="force-release-title">
      <Card class="w-full max-w-md space-y-4 border-destructive/40 bg-card p-5 shadow-2xl">
        <div class="flex items-start gap-3">
          <div class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-destructive/10 text-destructive">
            <TriangleAlert size={17} />
          </div>
          <div>
            <h3 id="force-release-title" class="text-sm font-semibold text-destructive">
              Force Release Port {pendingForceListener.port}?
            </h3>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
              <span class="font-semibold text-foreground">{pendingForceListener.server_name}</span> (PID {pendingForceListener.pid}) did not exit after the graceful stop request.
            </p>
          </div>
        </div>

        <div class="rounded-lg border border-destructive/20 bg-destructive/5 px-3 py-2.5 text-meta leading-relaxed text-destructive/90">
          Force release sends SIGKILL immediately. Unsaved work or open database transactions in this server process may not shut down cleanly.
        </div>

        <div class="flex justify-end gap-2 pt-1">
          <Button id="force-release-cancel" variant="ghost" size="sm" onclick={closePortDialogs}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            size="sm"
            disabled={developmentPortsStore.releasingId !== null}
            onclick={handleForceRelease}
          >
            Force Release
          </Button>
        </div>
      </Card>
    </div>
  {/if}
</div>
