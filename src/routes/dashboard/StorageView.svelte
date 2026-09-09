<script lang="ts">
  import { untrack } from 'svelte';
  import type { CategoryResult } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { memoryStore } from '../../lib/stores/memory.svelte';
  import { formatBytes, formatTimeAgo } from '../../lib/utils/format';
  import {
    tauriOpenStorageSettings,
    tauriShowInFileManager,
  } from '../../lib/utils/tauri';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import ProgressBar from '../../lib/components/ProgressBar.svelte';
  import CategoryCard from '../../lib/components/CategoryCard.svelte';
  import CleanResultModal from '../../lib/components/CleanResultModal.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import ScanFreshnessNotice from '../../lib/components/ScanFreshnessNotice.svelte';
  import ByteValue from '../../lib/components/ByteValue.svelte';
  import DeveloperArtifactsView from './DeveloperArtifactsView.svelte';
  import LargeFilesView from './LargeFilesView.svelte';
  import ApplicationsView from './ApplicationsView.svelte';
  import DiskView from './DiskView.svelte';
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
    FolderSearch,
    FileSearch,
    AppWindow,
  } from 'lucide-svelte';

  interface Props {
    onSelectCategory: (category: CategoryResult) => void;
    onOpenLargeFiles?: () => void;
    onOpenApplications?: () => void;
    onOpenDeveloperArtifacts?: () => void;
    onOpenDisks?: () => void;
    initialTab?: 'cleanup' | 'developer-artifacts' | 'large-files' | 'applications' | 'disks';
  }

  let {
    onSelectCategory,
    onOpenLargeFiles,
    onOpenApplications,
    onOpenDeveloperArtifacts,
    onOpenDisks,
    initialTab = 'cleanup',
  }: Props = $props();

  let activeSecondaryTab = $state<'cleanup' | 'developer-artifacts' | 'large-files' | 'applications' | 'disks'>('cleanup');

  $effect(() => {
    activeSecondaryTab = initialTab;
  });
  let disk = $derived(memoryStore.disk);
  let scan = $derived(scanStore.lastScan);
  let showResultModal = $state(false);
  let volumes = $derived(memoryStore.volumes);

  let safeSelectedBytes = $derived(scanStore.safeSelectedBytes);
  let rebuildSelectedBytes = $derived(scanStore.rebuildSelectedBytes);
  let manualSelectedBytes = $derived(scanStore.manualSelectedBytes);
  let hasRebuildSelected = $derived(scanStore.rebuildSelectedBytes > 0);

  $effect(() => {
    // One snapshot on activation and after a scan replaces the inventory.
    scanStore.lastScan?.scan_id;
    untrack(() => { void memoryStore.refreshDisk(); });
  });

  function handleCleanSelected() {
    scanStore.cleanSelected().then((res) => {
      if (res) showResultModal = true;
    });
  }

  function handleTabClick(tab: 'cleanup' | 'developer-artifacts' | 'large-files' | 'applications' | 'disks') {
    if (tab === 'developer-artifacts' && onOpenDeveloperArtifacts) {
      onOpenDeveloperArtifacts();
      return;
    }
    if (tab === 'large-files' && onOpenLargeFiles) {
      onOpenLargeFiles();
      return;
    }
    if (tab === 'applications' && onOpenApplications) {
      onOpenApplications();
      return;
    }
    if (tab === 'disks' && onOpenDisks) {
      onOpenDisks();
      return;
    }
    activeSecondaryTab = tab;
  }
</script>

<div class="space-y-6">
  <!-- Top Storage Header -->
  <div class="flex items-center justify-between pb-3 border-b border-border/60">
    <div class="flex items-center gap-3">
      <div class="h-9 w-9 rounded-lg bg-cyan-500/10 text-cyan-400 flex items-center justify-center shrink-0">
        <HardDrive size={20} />
      </div>
      <div>
        <h1 class="text-base font-semibold text-foreground tracking-tight">Storage</h1>
        <p class="text-xs text-muted-foreground mt-0.5">Primary storage capacity, cleanable development caches, and tools</p>
      </div>
    </div>
    <Button
      variant="outline"
      size="sm"
      disabled={scanStore.isScanning || scanStore.isCleaning}
      onclick={() => scanStore.runScan()}
      class="gap-1.5"
    >
      <RotateCw size={13} class={scanStore.isScanning ? 'animate-gentle-spin' : ''} />
      <span>{scanStore.isScanning ? 'Scanning…' : 'Scan Storage'}</span>
    </Button>
  </div>

  <!-- Secondary Navigation Tabs -->
  <div
    role="tablist"
    aria-label="Storage workflows"
    class="inline-flex items-center gap-1 p-1 rounded-lg bg-secondary/50 border border-border/50 text-muted-foreground overflow-x-auto max-w-full"
  >
    <button
      type="button"
      role="tab"
      aria-selected={activeSecondaryTab === 'cleanup'}
      onclick={() => handleTabClick('cleanup')}
      class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {activeSecondaryTab === 'cleanup' ? 'bg-card text-foreground shadow-xs font-semibold' : 'text-muted-foreground hover:text-foreground hover:bg-secondary/70'}"
    >
      <Trash2 size={13} />
      <span>Cleanup</span>
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={activeSecondaryTab === 'developer-artifacts'}
      onclick={() => handleTabClick('developer-artifacts')}
      class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {activeSecondaryTab === 'developer-artifacts' ? 'bg-card text-foreground shadow-xs font-semibold' : 'text-muted-foreground hover:text-foreground hover:bg-secondary/70'}"
    >
      <FolderSearch size={13} />
      <span>Developer Artifacts</span>
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={activeSecondaryTab === 'large-files'}
      onclick={() => handleTabClick('large-files')}
      class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {activeSecondaryTab === 'large-files' ? 'bg-card text-foreground shadow-xs font-semibold' : 'text-muted-foreground hover:text-foreground hover:bg-secondary/70'}"
    >
      <FileSearch size={13} />
      <span>Large Files</span>
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={activeSecondaryTab === 'applications'}
      onclick={() => handleTabClick('applications')}
      class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {activeSecondaryTab === 'applications' ? 'bg-card text-foreground shadow-xs font-semibold' : 'text-muted-foreground hover:text-foreground hover:bg-secondary/70'}"
    >
      <AppWindow size={13} />
      <span>Applications</span>
    </button>
    <button
      type="button"
      role="tab"
      aria-selected={activeSecondaryTab === 'disks'}
      onclick={() => handleTabClick('disks')}
      class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {activeSecondaryTab === 'disks' ? 'bg-card text-foreground shadow-xs font-semibold' : 'text-muted-foreground hover:text-foreground hover:bg-secondary/70'}"
    >
      <HardDrive size={13} />
      <span>Disks</span>
    </button>
  </div>

  {#if activeSecondaryTab === 'developer-artifacts'}
    <DeveloperArtifactsView onBack={() => (activeSecondaryTab = 'cleanup')} />
  {:else if activeSecondaryTab === 'large-files'}
    <LargeFilesView onBack={() => (activeSecondaryTab = 'cleanup')} />
  {:else if activeSecondaryTab === 'applications'}
    <ApplicationsView onBack={() => (activeSecondaryTab = 'cleanup')} />
  {:else if activeSecondaryTab === 'disks'}
    <DiskView
      onReviewCategory={(cat) => {
        activeSecondaryTab = 'cleanup';
        onSelectCategory(cat);
      }}
      onBack={() => (activeSecondaryTab = 'cleanup')}
    />
  {:else}
    <ScanFreshnessNotice />

    <!-- Storage & Cleanable Overview Card -->
    <Card class="p-6 bg-card/70 border-border/80 relative overflow-hidden space-y-6">
      <div class="flex flex-col md:flex-row md:items-center justify-between gap-6">
        <!-- Left: Primary Disk Space -->
        <div class="min-w-0 flex-1 space-y-2">
          <div class="flex flex-wrap justify-between items-baseline gap-x-3 gap-y-1">
            <span class="text-xs font-medium text-muted-foreground uppercase tracking-wider flex items-center gap-1.5">
              <HardDrive size={13} class="text-cyan-400" />
              Primary Storage
            </span>
            {#if disk}
              <span class="flex flex-wrap gap-x-1 font-mono tabular-nums text-sm font-semibold text-foreground">
                <ByteValue bytes={disk.used_bytes} />
                <span class="whitespace-nowrap">/ <ByteValue bytes={disk.total_bytes} /></span>
                <span class="whitespace-nowrap">({disk.percent_used?.toFixed(1) ?? '—'}%)</span>
              </span>
            {/if}
          </div>
          {#if disk}
            <ProgressBar value={disk.percent_used ?? 0} height="h-2.5" />
            <div class="flex flex-wrap justify-between gap-x-3 text-meta text-muted-foreground font-mono">
              <span>Free: <ByteValue bytes={disk.free_bytes} /></span>
              <span>Used: <ByteValue bytes={disk.used_bytes} /></span>
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
            <ByteValue bytes={scanStore.reclaimableBytes} />
          </div>
          <div class="text-meta text-muted-foreground">
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
            <span class="text-meta font-medium text-muted-foreground uppercase tracking-wider">Mounted Volumes</span>
            <button
              type="button"
              onclick={() => handleTabClick('disks')}
              class="text-meta text-muted-foreground hover:text-foreground underline underline-offset-2"
            >
              View in Disks
            </button>
          </div>
          <div class="grid grid-cols-1 sm:grid-cols-2 gap-2">
            {#each volumes as volume (volume.mount_point)}
              <div class="p-2.5 rounded-lg border border-border/50 bg-secondary/20 flex items-center justify-between text-xs">
                <div class="min-w-0 pr-2">
                  <div class="flex items-center gap-1.5">
                    <span class="font-medium truncate">{volume.name || volume.mount_point}</span>
                    {#if volume.is_primary}
                      <span class="px-1 py-0.2 rounded text-micro bg-success/10 text-success border border-success/20">Primary</span>
                    {/if}
                    {#if volume.is_removable}
                      <span class="px-1 py-0.2 rounded text-micro bg-secondary text-muted-foreground border border-border">External</span>
                    {/if}
                  </div>
                  <p class="flex flex-wrap gap-x-1 text-caption font-mono tabular-nums text-muted-foreground mt-0.5">
                    <ByteValue bytes={volume.used_bytes} />
                    <span class="whitespace-nowrap">/ <ByteValue bytes={volume.total_bytes} /></span>
                    <span class="whitespace-nowrap">({volume.percent_used != null ? `${volume.percent_used.toFixed(1)}%` : '—'})</span>
                  </p>
                </div>
                <Button
                  variant="ghost"
                  size="icon"
                  class="h-6 w-6 text-muted-foreground shrink-0"
                  title="Show in File Manager"
                  ariaLabel={`Show ${volume.name || volume.mount_point} in file manager`}
                  onclick={() => tauriShowInFileManager(volume.mount_point)}
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
            disabled={!scanStore.canClean}
            onclick={() => scanStore.selectAllSafe()}
            class="text-xs px-2.5"
          >
            <CheckSquare size={13} class="mr-1 text-success" />
            <span>Select Safe Only</span>
          </Button>

          <Button
            variant="ghost"
            size="sm"
            disabled={!scanStore.canClean}
            onclick={() => scanStore.deselectAll()}
            class="text-xs text-muted-foreground px-2.5"
          >
            <Square size={13} class="mr-1" />
            <span>Deselect All</span>
          </Button>

          <Button
            variant="ghost"
            size="sm"
            onclick={() => tauriOpenStorageSettings()}
            class="text-xs text-muted-foreground gap-1 px-2.5"
            title="Open storage settings"
            ariaLabel="Open storage settings"
          >
            <ExternalLink size={12} />
            <span>Storage Settings</span>
          </Button>
        </div>

        <div class="flex flex-col sm:flex-row sm:items-center md:flex-col md:items-end gap-2 shrink-0">
          {#if scanStore.selectedCount > 0}
            <div class="flex flex-wrap items-center gap-2 text-meta font-mono">
              <span class="whitespace-nowrap text-success font-medium">✓ {formatBytes(safeSelectedBytes)} Safe</span>
              {#if rebuildSelectedBytes > 0}
                <span class="whitespace-nowrap text-warning font-medium">↻ {formatBytes(rebuildSelectedBytes)} Rebuildable</span>
              {/if}
              {#if manualSelectedBytes > 0}
                <span class="whitespace-nowrap text-destructive font-medium">! {formatBytes(manualSelectedBytes)} Manual</span>
              {/if}
            </div>
          {/if}
          <Button
            variant="primary"
            size="md"
            disabled={!scanStore.canClean || scanStore.reclaimableBytes === 0}
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

    <!-- Categories Section -->
    <div class="space-y-3">
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold text-foreground tracking-tight">
          Storage Categories
        </h2>
        <div class="flex items-center gap-1 text-xs text-muted-foreground">
          <ShieldCheck size={14} class="text-success" />
          <span>Protected by Safety Engine</span>
        </div>
      </div>

      {#if scan}
        <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
          {#each scan.categories as categoryResult (categoryResult.category)}
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
  {/if}

  {#if showResultModal && scanStore.lastCleanResult}
    <CleanResultModal
      result={scanStore.lastCleanResult}
      onClose={() => (showResultModal = false)}
    />
  {/if}
</div>
