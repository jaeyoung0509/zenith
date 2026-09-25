<script lang="ts">
  import SegmentedTabs from '../../lib/components/SegmentedTabs.svelte';
  import CleanupReviewDialog from '../../lib/components/CleanupReviewDialog.svelte';
  import InlineNotice from '../../lib/components/InlineNotice.svelte';
  import ScanFreshnessNotice from '../../lib/components/ScanFreshnessNotice.svelte';
  import { onMount } from 'svelte';
  import type { CategoryResult, PlanPreview } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { platformCapabilitiesStore } from '../../lib/stores/platformCapabilities.svelte';
  import {
    tauriOpenStorageSettings,
  } from '../../lib/utils/tauri';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import ProgressBar from '../../lib/components/ProgressBar.svelte';
  import CategoryCard from '../../lib/components/CategoryCard.svelte';
  import StorageSummary from '../../lib/components/StorageSummary.svelte';
  import CleanResultModal from '../../lib/components/CleanResultModal.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import SelectionToolbar from '../../lib/components/SelectionToolbar.svelte';
  import LoadingSpinner from '../../lib/components/LoadingSpinner.svelte';
  import DeveloperArtifactsView from './DeveloperArtifactsView.svelte';
  import LargeFilesView from './LargeFilesView.svelte';
  import ApplicationsView from './ApplicationsView.svelte';
  import DiskView from './DiskView.svelte';
  import { restoreFocus } from '../../lib/utils/focus';
  import { isActionable, summarizeCategory } from '../../lib/utils/cleanup';
  import {
    RotateCw,
    Trash2,
    AlertCircle,
    HardDrive,
    ExternalLink,
    FolderSearch,
    FileSearch,
    AppWindow,
    Square,
  } from '@lucide/svelte';

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

  onMount(() => {
    void platformCapabilitiesStore.load();
  });

  let isApplicationsInspectable = $derived(
    platformCapabilitiesStore.isInspectable('installed_apps')
  );

  $effect(() => {
    if (initialTab === 'applications' && !isApplicationsInspectable) {
      activeSecondaryTab = 'cleanup';
    } else {
      activeSecondaryTab = initialTab;
    }
  });
  let scan = $derived(scanStore.lastScan);
  let orderedCategories = $derived(
    [...(scan?.categories ?? [])].sort((a, b) =>
      summarizeCategory(b.items).cleanable_bytes - summarizeCategory(a.items).cleanable_bytes
    )
  );
  let hasSelectedAction = $derived(
    scan?.categories.some(category => category.items.some(
      item => scanStore.selectedMap[item.id] && isActionable(item)
    )) ?? false
  );
  let showResultModal = $state(false);
  let review = $state<{ scanId: string; plan: PlanPreview } | null>(null);
  let isPreparingReview = $state(false);
  const storagePanelId = $props.id();
  const baseStorageTabs = [
    { id: 'cleanup', label: 'Cleanup', icon: Trash2 },
    { id: 'developer-artifacts', label: 'Developer Artifacts', icon: FolderSearch },
    { id: 'large-files', label: 'Large Files', icon: FileSearch },
    { id: 'applications', label: 'Applications', icon: AppWindow },
    { id: 'disks', label: 'Disks', icon: HardDrive },
  ];
  let storageTabs = $derived(
    baseStorageTabs.filter((tab) => tab.id !== 'applications' || isApplicationsInspectable)
  );

  async function handleCleanSelected() {
    if (!scan || !scanStore.canClean) return;
    const scanId = scan.scan_id;
    const items = scan.categories.flatMap(category => category.items)
      .filter(item => scanStore.selectedMap[item.id] && isActionable(item));
    isPreparingReview = true;
    try {
      const plan = await scanStore.prepareCleanup(items);
      if (plan && scanStore.lastScan?.scan_id === scanId && scanStore.canClean) {
        review = { scanId, plan };
      }
    } finally {
      isPreparingReview = false;
    }
  }

  function confirmCleanup() {
    if (!review || review.scanId !== scan?.scan_id || !scanStore.canClean) return;
    const plan = review.plan;
    review = null;
    // The review trigger becomes disabled while cleanup runs. After the
    // native dialog unmounts, move focus to the still-enabled active tab
    // instead of allowing WebKit to fall back to the tabpanel itself.
    queueMicrotask(() => restoreFocus());
    // Execute only the reviewed selection, even if another consumer selected
    // additional items while the review was open. Backend plans revalidate it.
    scanStore.executePreparedPlan(plan, true).then((res) => {
      if (res) {
        showResultModal = true;
      } else {
        restoreFocus();
      }
    });
  }

  function handleTabClick(tab: 'cleanup' | 'developer-artifacts' | 'large-files' | 'applications' | 'disks') {
    if (tab === 'applications' && !isApplicationsInspectable) {
      return;
    }
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

<div class="storage-workspace space-y-4">
  <!-- Top Storage Header -->
  <div class="flex flex-wrap gap-3 items-center justify-between">
    <div>
      <h1 class="text-title font-semibold text-foreground tracking-tight">Storage</h1>
      <p class="text-body text-muted-foreground mt-1">Make room for your next project.</p>
    </div>
    <Button
      variant="outline"
      size="sm"
      disabled={scanStore.isScanning || scanStore.isCleaning}
      onclick={() => scanStore.runScan()}
      class="gap-1.5"
      id="storage-scan-button"
      motion="paint"
    >
      <span class="inline-flex items-center justify-center shrink-0 w-3.5 h-3.5">
        {#if scanStore.isScanning}
          <LoadingSpinner size={13} />
        {:else}
          <RotateCw size={13} />
        {/if}
      </span>
      <span>{scanStore.isScanning ? 'Scanning…' : 'Scan Storage'}</span>
    </Button>
  </div>

  <!-- Secondary Navigation Tabs -->
  <SegmentedTabs
    appearance="underline"
    tabs={storageTabs}
    panelId={storagePanelId}
    activeTab={activeSecondaryTab}
    ariaLabel="Storage workflows"
    onSelect={(tab) => handleTabClick(tab as typeof activeSecondaryTab)}
  />

  {#if platformCapabilitiesStore.error !== null && platformCapabilitiesStore.capabilities === null}
    <InlineNotice
      variant="error"
      message={`Could not verify installed-application inspection: ${platformCapabilitiesStore.error}. Retry to restore the Applications tab.`}
      actionLabel="Retry"
      onAction={() => void platformCapabilitiesStore.load(true)}
    />
  {/if}

  <div
    class="space-y-4 outline-none focus:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-offset-2 rounded-xl"
    id={storagePanelId}
    role="tabpanel"
    aria-label={storageTabs.find(tab => tab.id === activeSecondaryTab)?.label}
    tabindex="0"
  >
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
    {#if !scanStore.isScanning && !scanStore.isCleaning && !scanStore.isRefreshingAfterClean}
      <StorageSummary />
    {/if}

    <!-- Scan Progress -->
    {#if scanStore.isScanning}
      <Card class="p-4 bg-secondary/60 border-primary/40 transition-colors duration-200">
        <div class="flex items-start justify-between gap-3" role="status" aria-live="polite">
          <div class="min-w-0 flex-1 space-y-1">
            <span class="text-xs font-medium text-foreground flex items-center gap-2">
              <LoadingSpinner size={13} />
              <span>{scanStore.isRefreshingAfterClean ? 'Updating storage after cleanup…' : scanStore.currentRoot ? `Reading ${scanStore.currentRoot.name}` : 'Scanning…'}</span>
            </span>
            {#if scanStore.currentRoot}
              <p class="font-mono text-caption text-muted-foreground truncate" title={scanStore.currentRoot.path}>
                {scanStore.currentRoot.path}
              </p>
            {/if}
            <p class="text-meta text-muted-foreground">
              {scanStore.foundItemCount} {scanStore.foundItemCount === 1 ? 'item' : 'items'} found so far
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            disabled={scanStore.isCancelling}
            onclick={() => void scanStore.cancelScan()}
            ariaLabel="Stop scan"
            title="Stop scan"
            class="gap-1.5 shrink-0"
          >
            <Square size={12} />
            <span>{scanStore.isCancelling ? 'Stopping…' : 'Stop'}</span>
          </Button>
        </div>
      </Card>
    {/if}

    <!-- Cleaning In Progress Bar -->
    {#if scanStore.isCleaning}
      <Card class="p-4 bg-secondary/60 border-primary/40 transition-colors duration-200">
        <div class="space-y-2" role="status" aria-live="polite">
          <div class="flex items-center justify-between text-xs">
            <span class="font-medium text-foreground flex items-center gap-2">
              <DeletingDots size="xs" />
              <span>Cleaning: {scanStore.cleanProgress.currentItem}</span>
            </span>
            <span class="font-mono text-muted-foreground font-semibold">
              {scanStore.cleanProgress.index} / {scanStore.cleanProgress.total} ({scanStore.cleanProgress.percent}%)
            </span>
          </div>
          <ProgressBar value={scanStore.cleanProgress.percent} height="h-2" color="bg-primary" />
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

    <!-- Scan freshness / remediation notice -->
    {#if !scanStore.isScanning && !scanStore.isCleaning && scan && scanStore.freshness !== 'fresh' && scanStore.freshness !== 'failed'}
      <ScanFreshnessNotice compact />
    {/if}

    <!-- Categories Section -->
    {#if !scanStore.isCleaning && !scanStore.isRefreshingAfterClean}
    <div class="space-y-3">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <h2 class="text-sm font-semibold text-foreground tracking-tight">Storage Categories</h2>
        {#if scan}
          <span class="text-meta text-muted-foreground">Largest cleanup first</span>
        {/if}
      </div>

      {#if scan}
        <div class="category-list rounded-xl border border-border bg-card">
          {#each orderedCategories as categoryResult (categoryResult.category)}
            <CategoryCard
              {categoryResult}
              onSelectCategory={(cat) => onSelectCategory(cat)}
            />
          {/each}
        </div>
      {:else}
        <div class="rounded-xl border border-border bg-card px-6 py-8 text-center space-y-2">
          {#if scanStore.isScanning}
            <LoadingSpinner size={24} class="mx-auto text-muted-foreground" />
            <p class="text-body text-muted-foreground">Scanning known development caches...</p>
          {:else}
            <HardDrive size={24} class="mx-auto mb-3 text-muted-foreground" aria-hidden="true" />
            <p class="text-sm font-medium text-foreground">Start with a storage scan</p>
            <p class="text-body text-muted-foreground">Find known caches, then choose what to review.</p>
          {/if}
        </div>
      {/if}
    </div>
    {/if}
    <!-- Review follows the list in both visual and keyboard order. -->
      {#if scan && !scanStore.isScanning && !scanStore.isCleaning && !scanStore.isRefreshingAfterClean}
      <div class="storage-selection">
      <SelectionToolbar
        selectedCount={scanStore.selectedCount}
        selectedBytes={scanStore.reclaimableBytes}
        manualCount={scanStore.manualSelectedCount}
        actionLabel="Review selected"
        onAction={handleCleanSelected}
        isActionDisabled={!scanStore.canClean || !hasSelectedAction || isPreparingReview}
        isActionLoading={scanStore.isCleaning || isPreparingReview}
        isSelectionDisabled={!scanStore.canClean}
      >
        {#snippet extraActions()}
          <Button
            variant="ghost"
            size="xs"
            onclick={() => tauriOpenStorageSettings()}
            class="text-muted-foreground"
            title="Open storage settings"
            ariaLabel="Open storage settings"
          >
            <ExternalLink size={12} />
            <span class="hidden lg:inline">Storage Settings</span>
          </Button>
        {/snippet}
      </SelectionToolbar>
      <p class="mt-1 text-meta text-muted-foreground">Nothing is removed until you confirm the review.</p>
      </div>
      {/if}

  {/if}

  </div>

  {#if review}
    <CleanupReviewDialog
      plan={review.plan}
      disabled={review.scanId !== scan?.scan_id || !scanStore.canClean}
      onCancel={() => (review = null)}
      onConfirm={confirmCleanup}
    />
  {/if}

  {#if showResultModal && scanStore.lastCleanResult}
    <CleanResultModal
      result={scanStore.lastCleanResult}
      onClose={() => (showResultModal = false)}
      returnFocusTargetId="storage-scan-button"
    />
  {/if}
</div>

<style>
  .storage-workspace { container-type: inline-size; }
  .category-list { padding: 0 4px; }
  .storage-selection {
    position: sticky;
    bottom: 0;
    z-index: 2;
    padding: 8px 0;
    background: hsl(var(--background));
  }
</style>
