<script lang="ts">
  import { onMount } from 'svelte';
  import type { ProcessMemory } from '../../lib/models/types';
  import { memoryStore } from '../../lib/stores/memory.svelte';
  import { formatBytes } from '../../lib/utils/format';
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
  } from 'lucide-svelte';

  onMount(() => {
    memoryStore.startPolling(2500);
    return () => {
      memoryStore.stopPolling();
    };
  });

  let memory = $derived(memoryStore.memory);
  let pendingProcess = $state<ProcessMemory | null>(null);

  async function terminatePending(force: boolean) {
    if (!pendingProcess) return;
    const name = pendingProcess.name;
    pendingProcess = null;
    await memoryStore.terminateProcessGroup(name, force);
  }

  const pressureColors = {
    normal: 'text-emerald-500 bg-emerald-500/10 border-emerald-500/20',
    warning: 'text-amber-500 bg-amber-500/10 border-amber-500/20',
    critical: 'text-rose-500 bg-rose-500/10 border-rose-500/20',
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
          <h2 class="text-base font-semibold text-foreground tracking-tight">Memory Inspector</h2>
          {#if memory}
            <div class="px-2 py-0.5 rounded-full text-[10px] font-mono font-medium border {pressureColors[memory.pressure]}">
              Estimated: {memory.pressure.toUpperCase()}
            </div>
          {/if}
        </div>
        <p class="text-xs text-muted-foreground mt-0.5">
          Real-time macOS memory pressure, compressed memory, swap usage, and developer processes.
        </p>
      </div>
    </div>

    <Button
      variant="outline"
      size="sm"
      disabled={memoryStore.isLoading}
      onclick={() => memoryStore.refreshMemory()}
      class="gap-1.5 text-xs"
    >
      <RotateCw size={13} class={memoryStore.isLoading ? 'animate-spin' : ''} />
      <span>Refresh</span>
    </Button>
  </div>

  {#if memoryStore.error}
    <div class="rounded-xl border border-red-500/20 bg-red-500/5 px-4 py-3 text-xs text-red-500">{memoryStore.error}</div>
  {:else if memoryStore.lastAction}
    <div class="rounded-xl border border-emerald-500/20 bg-emerald-500/5 px-4 py-3 text-xs text-emerald-600 dark:text-emerald-400">{memoryStore.lastAction} macOS may retain some memory as reusable cache.</div>
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
          color={memory.pressure === 'critical' ? 'bg-rose-500' : memory.pressure === 'warning' ? 'bg-amber-500' : 'bg-emerald-500'}
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
        <p class="text-[11px] text-muted-foreground mt-1">
          Secondary disk paging memory usage.
        </p>
      </Card>
    </div>

    <!-- Top Developer Processes Table -->
    <div class="space-y-3 pt-2">
      <div class="flex items-center justify-between">
        <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
          Top Resource Consuming Processes
        </h3>
        <span class="text-[11px] text-muted-foreground font-mono">
          Updating every 2.5s
        </span>
      </div>

      <div class="border border-border/80 rounded-xl overflow-hidden bg-card/70 divide-y divide-border/60">
        {#each memory.top_processes as proc}
          <div class="group flex items-center justify-between p-3 text-xs hover:bg-secondary/30 transition-colors">
            <div class="flex items-center gap-3">
              <div class="font-mono text-[11px] text-muted-foreground w-12">
                PID {proc.pid}
              </div>
              <div>
                <span class="font-medium text-foreground">{proc.name}</span>
                {#if proc.process_count > 1}
                  <span class="text-muted-foreground ml-1.5 text-[11px]">
                    ({proc.process_count} instances)
                  </span>
                {/if}
              </div>
            </div>

            <div class="flex items-center gap-4">
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
    </div>
  {:else}
    <div class="py-16 text-center text-xs text-muted-foreground space-y-2">
      <RotateCw size={20} class="animate-spin mx-auto opacity-50" />
      <p>Reading macOS memory statistics...</p>
    </div>
  {/if}

  {#if pendingProcess}
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-4 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="terminate-title">
      <Card class="w-full max-w-md space-y-4 border-border bg-card p-5 shadow-2xl">
        <div class="flex items-start gap-3">
          <div class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-amber-500/10 text-amber-500">
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
