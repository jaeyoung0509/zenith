<script lang="ts">
  import { onMount } from 'svelte';
  import type { CategoryResult, DiskVolume } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { memoryStore } from '../../lib/stores/memory.svelte';
  import { formatBytes, formatTimeAgo } from '../../lib/utils/format';
  import {
    tauriGetDiskVolumes,
    tauriOpenDiskUtility,
    tauriRevealInFinder,
  } from '../../lib/utils/tauri';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import ProgressBar from '../../lib/components/ProgressBar.svelte';
  import CategoryCard from '../../lib/components/CategoryCard.svelte';
  import CleanResultModal from '../../lib/components/CleanResultModal.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import {
    RotateCw,
    Trash2,
    CheckSquare,
    Square,
    ShieldCheck,
    AlertCircle,
    HardDrive,
    ExternalLink,
    FolderOpen,
  } from 'lucide-svelte';

  interface Props {
    onSelectCategory: (category: CategoryResult) => void;
  }

  let { onSelectCategory }: Props = $props();

  let disk = $derived(memoryStore.disk);
  let scan = $derived(scanStore.lastScan);
  let showResultModal = $state(false);
  let volumes = $state<DiskVolume[]>([]);
  let isLoadingVolumes = $state(false);

  let safeSelectedBytes = $derived(scanStore.safeSelectedBytes);
  let rebuildSelectedBytes = $derived(scanStore.rebuildSelectedBytes);
  let manualSelectedBytes = $derived(scanStore.manualSelectedBytes);
  let hasRebuildSelected = $derived(scanStore.rebuildSelectedBytes > 0);

  async function loadVolumes() {
    isLoadingVolumes = true;
    try {
      volumes = await tauriGetDiskVolumes();
    } catch {
      // Ignore or fallback
    } finally {
      isLoadingVolumes = false;
    }
  }

  onMount(() => {
    void loadVolumes();
  });

  function handleCleanSelected() {
    scanStore.cleanSelected().then((res) => {
      if (res) showResultModal = true;
    });
  }
</script>

<div class="space-y-6">
  <!-- Storage & Cleanable Overview Card -->
  <Card class="p-6 bg-card/70 border-border/80 relative overflow-hidden space-y-6">
    <div class="flex flex-col md:flex-row md:items-center justify-between gap-6">
      <!-- Left: Primary Disk Space -->
      <div class="flex-1 space-y-2">
        <div class="flex justify-between items-baseline">
          <span class="text-xs font-medium text-muted-foreground uppercase tracking-wider flex items-center gap-1.5">
            <HardDrive size={13} class="text-cyan-400" />
            Mac Primary Storage
          </span>
          {#if disk}
            <span class="whitespace-nowrap font-mono text-sm font-semibold text-foreground">
              {formatBytes(disk.used_bytes)} / {formatBytes(disk.total_bytes)} ({disk.percent_used?.toFixed(1) ?? '—'}%)
            </span>
          {/if}
        </div>
        {#if disk}
          <ProgressBar value={disk.percent_used ?? 0} height="h-2.5" />
          <div class="flex justify-between text-[11px] text-muted-foreground font-mono">
            <span>Free: {formatBytes(disk.free_bytes)}</span>
            <span>Used: {formatBytes(disk.used_bytes)}</span>
          </div>
        {/if}
      </div>

      <!-- Divider -->
      <div class="hidden md:block w-px h-16 bg-border/60"></div>

      <!-- Right: Reclaimable Space -->
      <div class="space-y-1 md:text-right min-w-[200px]">
        <span class="text-xs font-medium text-muted-foreground uppercase tracking-wider">
          Selected Reclaimable
        </span>
        <div class="whitespace-nowrap text-3xl font-bold font-mono text-foreground">
          {formatBytes(scanStore.reclaimableBytes)}
        </div>
        <div class="text-[11px] text-muted-foreground">
          {#if scan}
            <span>Last scan {formatTimeAgo(scan.finished_at)}</span>
          {:else}
            <span>No scan completed yet</span>
          {/if}
        </div>
      </div>
    </div>

    <!-- Mounted Volumes (if multiple or external attached) -->
    {#if volumes.length > 1}
      <div class="pt-3 border-t border-border/40 space-y-2">
        <div class="flex items-center justify-between">
          <span class="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">Mounted Volumes</span>
        </div>
        <div class="grid grid-cols-1 sm:grid-cols-2 gap-2">
          {#each volumes as volume (volume.mount_point)}
            <div class="p-2.5 rounded-lg border border-border/50 bg-secondary/20 flex items-center justify-between text-xs">
              <div class="min-w-0 pr-2">
                <div class="flex items-center gap-1.5">
                  <span class="font-medium truncate">{volume.name || volume.mount_point}</span>
                  {#if volume.is_primary}
                    <span class="px-1 py-0.2 rounded text-[9px] bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">Primary</span>
                  {/if}
                  {#if volume.is_removable}
                    <span class="px-1 py-0.2 rounded text-[9px] bg-secondary text-muted-foreground border border-border">External</span>
                  {/if}
                </div>
                <p class="whitespace-nowrap text-[10px] font-mono text-muted-foreground mt-0.5">
                  {formatBytes(volume.used_bytes)} / {formatBytes(volume.total_bytes)} ({volume.percent_used != null ? `${volume.percent_used.toFixed(0)}%` : '—'})
                </p>
              </div>
              <Button
                variant="ghost"
                size="icon"
                class="h-6 w-6 text-muted-foreground shrink-0"
                title="Reveal in Finder"
                onclick={() => tauriRevealInFinder(volume.mount_point)}
              >
                <FolderOpen size={12} />
              </Button>
            </div>
          {/each}
        </div>
      </div>
    {/if}

    <!-- Action Toolbar -->
    <div class="pt-4 border-t border-border/60 flex flex-col md:flex-row md:items-center justify-between gap-3">
      <div class="flex flex-wrap items-center gap-1.5">
        <Button
          variant="ghost"
          size="sm"
          disabled={scanStore.isCleaning}
          onclick={() => scanStore.selectAllSafe()}
          class="text-xs px-2.5"
        >
          <CheckSquare size={13} class="mr-1 text-emerald-500" />
          <span>Select Safe Only</span>
        </Button>

        <Button
          variant="ghost"
          size="sm"
          disabled={scanStore.isCleaning}
          onclick={() => scanStore.deselectAll()}
          class="text-xs text-muted-foreground px-2.5"
        >
          <Square size={13} class="mr-1" />
          <span>Deselect All</span>
        </Button>

        <Button
          variant="ghost"
          size="sm"
          onclick={() => tauriOpenDiskUtility()}
          class="text-xs text-muted-foreground gap-1 px-2.5"
          title="Open macOS Disk Utility"
          ariaLabel="Open macOS Disk Utility"
        >
          <ExternalLink size={12} />
          <span>Disk Utility</span>
        </Button>
      </div>

      <div class="flex flex-col sm:flex-row sm:items-center md:flex-col md:items-end gap-2 shrink-0">
        {#if scanStore.selectedCount > 0}
          <div class="flex flex-wrap items-center gap-2 text-[11px] font-mono">
            <span class="whitespace-nowrap text-emerald-500 font-medium">✓ {formatBytes(safeSelectedBytes)} Safe</span>
            {#if rebuildSelectedBytes > 0}
              <span class="whitespace-nowrap text-amber-500 font-medium">↻ {formatBytes(rebuildSelectedBytes)} Rebuildable</span>
            {/if}
            {#if manualSelectedBytes > 0}
              <span class="whitespace-nowrap text-rose-400 font-medium">! {formatBytes(manualSelectedBytes)} Manual</span>
            {/if}
          </div>
        {/if}
        <Button
          variant="primary"
          size="md"
          disabled={scanStore.isScanning || scanStore.isCleaning || scanStore.reclaimableBytes === 0}
          onclick={handleCleanSelected}
          class="gap-2 px-5 min-w-[130px]"
        >
          {#if scanStore.isCleaning}
            <DeletingDots size="sm" />
            <span>Cleaning…</span>
          {:else}
            <Trash2 size={14} />
            <span>{hasRebuildSelected ? 'Review & Clean' : 'Clean Safely'}</span>
          {/if}
        </Button>
      </div>
    </div>
  </Card>

  <!-- Cleaning In Progress Bar -->
  {#if scanStore.isCleaning}
    <Card class="p-4 bg-secondary/60 border-primary/40 shadow-sm transition-all duration-200">
      <div class="space-y-2">
        <div class="flex items-center justify-between text-xs">
          <span class="font-medium text-foreground flex items-center gap-2">
            <DeletingDots size="xs" />
            <span>Cleaning: {scanStore.cleanProgress.currentItem}</span>
          </span>
          <span class="font-mono text-muted-foreground font-semibold">
            {scanStore.cleanProgress.index} / {scanStore.cleanProgress.total} ({scanStore.cleanProgress.percent}%)
          </span>
        </div>
        <ProgressBar value={scanStore.cleanProgress.percent} height="h-2" color="bg-primary" animated={true} />
      </div>
    </Card>
  {/if}

  <!-- Error Alert -->
  {#if scanStore.error}
    <div class="p-3.5 rounded-xl bg-destructive/15 border border-destructive/30 text-destructive flex items-center gap-2.5 text-xs">
      <AlertCircle size={16} class="shrink-0" />
      <span>{scanStore.error}</span>
    </div>
  {/if}

  <!-- Categories Grid -->
  <div class="space-y-3">
    <div class="flex items-center justify-between">
      <h2 class="text-sm font-semibold text-foreground tracking-tight">
        Storage Categories
      </h2>
      <div class="flex items-center gap-1 text-xs text-muted-foreground">
        <ShieldCheck size={14} class="text-emerald-500" />
        <span>Protected by Safety Engine</span>
      </div>
    </div>

    {#if scan}
      <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
        {#each scan.categories as categoryResult}
          <CategoryCard
            {categoryResult}
            onSelectCategory={(cat) => onSelectCategory(cat)}
          />
        {/each}
      </div>
    {:else}
      <div class="py-12 text-center text-muted-foreground text-sm space-y-3">
        <RotateCw size={24} class="animate-gentle-spin mx-auto opacity-50" />
        <p>Scanning known development caches...</p>
      </div>
    {/if}
  </div>

  {#if showResultModal && scanStore.lastCleanResult}
    <CleanResultModal
      result={scanStore.lastCleanResult}
      onClose={() => (showResultModal = false)}
    />
  {/if}
</div>
