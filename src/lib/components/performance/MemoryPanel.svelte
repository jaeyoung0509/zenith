<script lang="ts">
  import { restoreFocus } from '../../utils/focus';
  import type { ProcessMemory } from '../../models/types';
  import { memoryStore } from '../../stores/memory.svelte';
  import { platformCapabilitiesStore } from '../../stores/platformCapabilities.svelte';
  import { formatBytes, formatTimeAgo } from '../../utils/format';
  import { withMinimumDuration } from '../../utils/async';
  import { filterProcesses } from '../../utils/memory';
  import { memoryPressureLabel } from '../../utils/systemReadings';
  import Button from '../Button.svelte';
  import Card from '../Card.svelte';
  import Badge from '../Badge.svelte';
  import ProgressBar from '../ProgressBar.svelte';
  import ByteValue from '../ByteValue.svelte';
  import InlineNotice from '../InlineNotice.svelte';
  import {
    RotateCw,
    Layers,
    Cpu,
    Database,
    LogOut,
    TriangleAlert,
    Search,
    X,
  } from '@lucide/svelte';

  let { compact = false }: { compact?: boolean } = $props();
  let terminationDialog = $state<HTMLDialogElement>();
  let returnFocus: HTMLElement | null = null;
  $effect(() => {
    const dialog = terminationDialog;
    if (!dialog || !pendingProcess) return;
    dialog.showModal();
    return () => {
      dialog.close();
      restoreFocus(returnFocus);
    };
  });

  let memory = $derived(memoryStore.memory);
  let pendingProcess = $state<ProcessMemory | null>(null);
  let forceAuthorized = $state(false);
  let isRefreshing = $state(false);
  let searchQuery = $state('');
  let isWindows = $derived(platformCapabilitiesStore.capabilities?.platform === 'windows');

  function openTerminationDialog(proc: ProcessMemory) {
    returnFocus = document.activeElement as HTMLElement | null;
    pendingProcess = proc;
    forceAuthorized = isWindows;
  }

  let filteredProcesses = $derived(
    filterProcesses(memory?.top_processes ?? [], searchQuery)
  );

  let topReclaimableApp = $derived(
    memory?.top_processes.find((p) => p.can_terminate && p.memory_bytes > 400 * 1024 * 1024)
  );

  /**
   * One shared presentation of the system's own pressure verdict. Pressure is
   * the kernel's assessment of memory demand; it is not the used percentage, so
   * the two readings are never collapsed into one.
   */
  const pressureStyles = {
    normal: { chip: 'border-success/25 bg-success/10 text-success', dot: 'bg-success' },
    warning: { chip: 'border-warning/25 bg-warning/10 text-warning', dot: 'bg-warning' },
    critical: { chip: 'border-destructive/25 bg-destructive/10 text-destructive', dot: 'bg-destructive' },
  } as const;

  let pressureStyle = $derived(pressureStyles[memory?.pressure ?? 'normal']);

  /** Operational advice, and only while the system itself reports pressure. */
  let pressureAdvice = $derived.by(() => {
    if (!memory || memory.pressure === 'normal') return null;
    if (topReclaimableApp) {
      return `Closing ${topReclaimableApp.name} could recover ~${formatBytes(topReclaimableApp.memory_bytes)}.`;
    }
    return 'Consider closing background developer apps to reduce memory demand.';
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

  async function terminatePending(mode: 'graceful' | 'force') {
    if (!pendingProcess) return;
    const proc = pendingProcess;
    const leaseId = proc.termination_lease_id;
    if (!leaseId) {
      memoryStore.error = `Could not terminate ${proc.name}: termination snapshot expired; refresh and try again.`;
      pendingProcess = null;
      return;
    }
    pendingProcess = null;
    forceAuthorized = false;
    const result = await memoryStore.terminateMemoryGroup(leaseId, mode, proc.name);
    // Preserve the exact force-authorized lease returned by the graceful
    // attempt; the store refresh intentionally creates ordinary leases.
    if (result?.outcome === 'still_listening' && result.fresh_lease_id) {
      pendingProcess = { ...proc, termination_lease_id: result.fresh_lease_id };
      forceAuthorized = true;
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      if (pendingProcess) {
        pendingProcess = null;
        forceAuthorized = false;
      }
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="space-y-5">
  {#if memoryStore.error}
    <InlineNotice
      variant="destructive"
      title="Memory Error"
      message={memoryStore.error}
      onDismiss={() => (memoryStore.error = null)}
    />
  {:else if memoryStore.lastAction}
    <InlineNotice
      variant="success"
      title="Memory Action Complete"
      message={`${memoryStore.lastAction} The operating system may retain some memory as reusable cache.`}
      onDismiss={() => (memoryStore.lastAction = null)}
    />
  {/if}

  {#if memory}
    {#if !compact}
    <!-- Pressure first: the system's own verdict, before any percentage. -->
    <Card class="space-y-3">
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div class="min-w-0 space-y-1">
          <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
            <span
              class="inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-caption font-medium {pressureStyle.chip}"
            >
              <span class="h-1.5 w-1.5 rounded-full {pressureStyle.dot}"></span>
              <span>Memory pressure: {memoryPressureLabel(memory.pressure)}</span>
            </span>
            <span class="text-caption text-muted-foreground">Updated {formatTimeAgo(memory.timestamp)}</span>
          </div>
          <p class="max-w-prose text-meta text-muted-foreground">
            Pressure is the system's own assessment of memory demand, separate from the share of memory in use.
          </p>
          {#if pressureAdvice}
            <p class="text-meta text-muted-foreground">{pressureAdvice}</p>
          {/if}
        </div>

        <Button
          variant="outline"
          size="sm"
          disabled={isRefreshing || memoryStore.isLoading}
          onclick={handleRefresh}
          class="gap-1.5"
        >
          <RotateCw size={13} class={isRefreshing || memoryStore.isLoading ? 'animate-gentle-spin' : ''} />
          <span>Refresh</span>
        </Button>
      </div>

      <!-- Supporting facts only: no second verdict. -->
      <div class="memory-gauges grid gap-4 border-t border-border pt-3">
        <div class="space-y-1.5">
          <div class="flex items-center justify-between gap-2 text-meta font-medium text-muted-foreground">
            <span>Physical memory</span>
            <Cpu size={15} aria-hidden="true" />
          </div>
          <div class="min-h-16 text-metric font-semibold text-foreground">
            <ByteValue bytes={memory.used_bytes} />
            <div class="whitespace-nowrap text-meta font-normal text-muted-foreground">of <ByteValue bytes={memory.total_bytes} /> used</div>
          </div>
          <ProgressBar
            value={(memory.used_bytes / memory.total_bytes) * 100}
            height="h-2"
            color="bg-primary"
          />
          <div class="flex flex-wrap justify-between gap-x-3 text-caption text-muted-foreground">
            <span>Available: <ByteValue bytes={memory.available_bytes} /></span>
            <span>Free: <ByteValue bytes={memory.free_bytes} /></span>
          </div>
        </div>

        <div class="space-y-1.5">
          <div class="flex items-center justify-between gap-2 text-meta font-medium text-muted-foreground">
            <span>Swap used</span>
            <Database size={15} aria-hidden="true" />
          </div>
          <div class="min-h-16 text-metric font-semibold text-foreground">
            <ByteValue bytes={memory.swap_used_bytes} />
            {#if memory.swap_total_bytes > 0}
              <div class="whitespace-nowrap text-meta font-normal text-muted-foreground">of <ByteValue bytes={memory.swap_total_bytes} /> total</div>
            {/if}
          </div>
          {#if memory.swap_total_bytes > 0}
            <ProgressBar
              value={(memory.swap_used_bytes / memory.swap_total_bytes) * 100}
              height="h-2"
              color="bg-primary"
            />
          {/if}
        </div>

        <div class="space-y-1.5">
          <div class="flex items-center justify-between gap-2 text-meta font-medium text-muted-foreground">
            <span>Compressed memory</span>
            <Layers size={15} aria-hidden="true" />
          </div>
          <div class="min-h-16 text-metric font-semibold text-foreground">
            <ByteValue bytes={memory.compressed_bytes} />
          </div>
          <p class="text-caption text-muted-foreground">
            In-memory compression can reduce slower disk swap activity.
          </p>
        </div>
      </div>
    </Card>

    {/if}
    <!-- Top Developer Processes Table -->
    <div class="space-y-3">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="flex items-center gap-2">
          <h3 class="text-meta font-semibold uppercase tracking-wider text-muted-foreground">
            {compact ? 'Apps using memory' : 'Top Resource Consuming Processes'}
          </h3>
          <span class="rounded bg-secondary px-1.5 py-0.5 font-mono text-caption text-muted-foreground">
            2.5s live
          </span>
        </div>

        <div class="relative w-full sm:w-64">
          <Search size={14} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <input
            type="text"
            bind:value={searchQuery}
            placeholder="Search process, PID, or parent…"
            aria-label="Search processes by name, PID, or parent process"
            class="h-8 w-full rounded-lg border border-border bg-card pl-8 pr-7 text-meta text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
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

      <p class="text-meta text-muted-foreground">
        Zenith observes these processes in the system snapshot, ranked by memory. Quit is offered only for processes Zenith has verified it may stop.
      </p>

      {#if filteredProcesses.length > 0}
        <!-- One shared surface, one row per process. -->
        <div class="surface overflow-hidden divide-y divide-border">
          {#each filteredProcesses as proc (proc.name)}
            <div
              data-process-row
              class="group flex min-h-16 items-center justify-between gap-3 p-3 text-meta transition-ui hover:bg-secondary/40"
            >
              <div class="flex min-w-0 items-center gap-3 pr-2">
                <div
                  class="w-12 shrink-0 font-mono text-caption text-muted-foreground"
                  title={proc.pids && proc.pids.length > 1 ? `PIDs: ${proc.pids.join(', ')}` : undefined}
                >
                  PID {proc.pid}
                </div>
                <div class="min-w-0">
                  <!-- Korean app names break at their own word boundaries, never mid-word. -->
                  <span class="font-medium text-foreground break-keep [overflow-wrap:anywhere]">{proc.name}</span>
                  {#if proc.process_count > 1}
                    <span class="ml-1.5 text-caption text-muted-foreground">
                      ({proc.process_count} instances)
                    </span>
                  {/if}
                  {#if proc.parent_process_names?.length}
                    <span class="ml-1.5 text-caption text-muted-foreground">
                      parent: {proc.parent_process_names.join(', ')}
                    </span>
                  {/if}
                  {#if proc.ownership === 'zenith_child'}
                    <Badge variant="secondary" class="ml-1.5">Started by Zenith</Badge>
                  {/if}
                </div>
              </div>

              <div class="flex shrink-0 items-center gap-4">
                <ByteValue bytes={proc.memory_bytes} class="w-[10ch] text-right font-semibold text-foreground" />
                <div class="w-16 shrink-0">
                {#if proc.can_terminate && proc.termination_lease_id}
                  <Button
                    variant="outline"
                    size="sm"
                    class="gap-1.5 opacity-70 group-hover:opacity-100"
                    disabled={memoryStore.terminating !== null}
                    onclick={() => openTerminationDialog(proc)}
                  >
                    <LogOut size={12} />
                    Quit
                  </Button>
                {/if}
                </div>
              </div>
            </div>
          {/each}
        </div>
      {:else if searchQuery.trim()}
        <div class="surface space-y-2 p-8 text-center">
          <p class="text-meta text-muted-foreground">No processes matching "{searchQuery}"</p>
          <Button variant="ghost" size="sm" onclick={() => (searchQuery = '')} class="text-meta">
            Clear Search
          </Button>
        </div>
      {:else}
        <div class="surface p-8 text-center text-meta text-muted-foreground">
          No high-memory processes detected.
        </div>
      {/if}
    </div>

  {:else}
    <div class="space-y-2 py-16 text-center text-meta text-muted-foreground">
      <RotateCw size={20} class="mx-auto animate-gentle-spin opacity-50" aria-hidden="true" />
      <p>Reading system memory statistics...</p>
    </div>
  {/if}

  <!-- Quit Process Group Modal -->
  {#if pendingProcess}
    <dialog bind:this={terminationDialog} class="m-auto w-[calc(100%-2rem)] max-w-md overflow-visible border-0 bg-transparent p-0 text-foreground backdrop:bg-background/80" aria-labelledby="terminate-title" oncancel={() => { pendingProcess = null; forceAuthorized = false; }}>
      <Card class="w-full max-w-md space-y-4 border-border bg-card p-5 shadow-2xl">
        <div class="flex items-start gap-3">
          <div class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-warning/10 text-warning">
            <TriangleAlert size={17} />
          </div>
          <div>
            <h3 id="terminate-title" class="text-body font-semibold">Quit {pendingProcess.name}?</h3>
            <p class="mt-1 text-meta leading-relaxed text-muted-foreground">
              This group contains {pendingProcess.process_count} processes using approximately {formatBytes(pendingProcess.memory_bytes)}. Unsaved work, active downloads, or running tasks may be lost.
            </p>
          </div>
        </div>

        <div class="rounded-lg border border-border bg-secondary/60 px-3 py-2.5 text-meta leading-relaxed text-muted-foreground">
          {#if isWindows}
            Windows does not provide a safe generic graceful action for this process group. Force Quit stops every verified process immediately.
          {:else if forceAuthorized}
            The normal quit request did not stop every process. Force Quit immediately stops only the members that still match the verified snapshot.
          {:else}
            Try normal Quit first. Force Quit becomes available only if the verified processes do not respond.
          {/if}
        </div>

        <div class="flex justify-end gap-2 pt-1">
          <Button variant="ghost" size="sm" onclick={() => { pendingProcess = null; forceAuthorized = false; }}>Cancel</Button>
          {#if !forceAuthorized}
            <Button variant="outline" size="sm" onclick={() => terminatePending('graceful')}>Quit Normally</Button>
          {:else}
            <Button variant="destructive" size="sm" onclick={() => terminatePending('force')}>Force Quit</Button>
          {/if}
        </div>
      </Card>
    </dialog>
  {/if}
</div>

<style>
  .memory-gauges {
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 12rem), 1fr));
  }
</style>
