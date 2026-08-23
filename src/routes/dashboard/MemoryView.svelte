<script lang="ts">
  import { onMount } from 'svelte';
  import type { ProcessMemory } from '../../lib/models/types';
  import { memoryStore } from '../../lib/stores/memory.svelte';
  import { formatBytes } from '../../lib/utils/format';
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
    Zap,
    LogOut,
    TriangleAlert,
    Search,
    X,
  } from 'lucide-svelte';

  onMount(() => {
    function updatePolling() {
      if (typeof document !== 'undefined' && document.visibilityState === 'visible') {
        memoryStore.startPolling(2500);
      } else {
        memoryStore.stopPolling();
      }
    }

    updatePolling();
    document.addEventListener('visibilitychange', updatePolling);

    return () => {
      document.removeEventListener('visibilitychange', updatePolling);
      memoryStore.stopPolling();
    };
  });

  let memory = $derived(memoryStore.memory);
  let pendingProcess = $state<ProcessMemory | null>(null);
  let isRefreshing = $state(false);
  let searchQuery = $state('');

  let filteredProcesses = $derived(
    filterProcesses(memory?.top_processes ?? [], searchQuery)
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
    await withMinimumDuration(memoryStore.refreshMemory(), 600);
    isRefreshing = false;
  }

  async function terminatePending(force: boolean) {
    if (!pendingProcess) return;
    const name = pendingProcess.name;
    pendingProcess = null;
    await memoryStore.terminateProcessGroup(name, force);
  }

  const pressureColors = {
    normal: 'text-success bg-success/10 border-success/20',
    warning: 'text-warning bg-warning/10 border-warning/20',
    critical: 'text-destructive bg-destructive/10 border-destructive/20',
  };
</script>

<div class="space-y-6">
  <!-- Header -->
  <div class="flex items-center justify-between pb-3 border-b border-border/60">
    <div class="flex items-center gap-3">
      <div class="h-9 w-9 rounded-lg bg-emerald-500/10 text-emerald-400 flex items-center justify-center">
        <Activity size={20} />
      </div>
      <div>
        <div class="flex items-center gap-2">
          <h2 class="text-base font-semibold text-foreground tracking-tight">{memoryHealthTitle}</h2>
          {#if memory}
            <div class="px-2.5 py-0.5 rounded-full text-[10px] font-mono font-medium border flex items-center gap-1.5 {pressureColors[memory.pressure]}">
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
        <div class="flex justify-between text-[10px] text-muted-foreground font-mono">
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
        <p class="text-[11px] text-muted-foreground mt-1">
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
        <p class="text-[11px] text-muted-foreground mt-1">
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
          <span class="text-[10px] text-muted-foreground font-mono bg-secondary/80 px-1.5 py-0.5 rounded">
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
                  class="font-mono text-[11px] text-muted-foreground w-12 shrink-0"
                  title={proc.pids && proc.pids.length > 1 ? `PIDs: ${proc.pids.join(', ')}` : undefined}
                >
                  PID {proc.pid}
                </div>
                <div class="min-w-0 truncate">
                  <span class="font-medium text-foreground">{proc.name}</span>
                  {#if proc.process_count > 1}
                    <span class="text-muted-foreground ml-1.5 text-[11px]">
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
  {:else}
    <div class="py-16 text-center text-xs text-muted-foreground space-y-2">
      <RotateCw size={20} class="animate-gentle-spin mx-auto opacity-50" />
      <p>Reading macOS memory statistics...</p>
    </div>
  {/if}

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

        <div class="rounded-lg border border-border/70 bg-secondary/40 px-3 py-2.5 text-[11px] leading-relaxed text-muted-foreground">
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
</div>
