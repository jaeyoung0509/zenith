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
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import ProgressBar from '../../lib/components/ProgressBar.svelte';
  import CategoryCard from '../../lib/components/CategoryCard.svelte';
  import StorageSummary from '../../lib/components/StorageSummary.svelte';
  import CleanResultModal from '../../lib/components/CleanResultModal.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import SelectionToolbar from '../../lib/components/SelectionToolbar.svelte';
  import DeveloperArtifactsView from './DeveloperArtifactsView.svelte';
  import LargeFilesView from './LargeFilesView.svelte';
  import ApplicationsView from './ApplicationsView.svelte';
  import DiskView from './DiskView.svelte';
  import { restoreFocus } from '../../lib/utils/focus';
  import { isActionable, summarizeCategory } from '../../lib/utils/cleanup';
  import {
    RotateCw,
    Trash2,
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
  <PageHeader title="Storage" subtitle="Make room for your next project." icon={HardDrive}>
    {#snippet actions()}
      {#if activeSecondaryTab === 'cleanup'}
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
          <RotateCw size={13} />
        </span>
        <span>{scanStore.isScanning ? 'Scanning…' : 'Scan Storage'}</span>
      </Button>
      {/if}
    {/snippet}
  </PageHeader>

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
      <Card class="p-5 bg-card border-border transition-colors duration-200">
        <div class="space-y-4" role="status" aria-live="polite" aria-busy="true">
        <div class="flex items-start justify-between gap-4">
          <div class="min-w-0 flex items-start gap-3">
            <span class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-accent text-primary" aria-hidden="true">
              <DeletingDots size="sm" />
            </span>
            <div class="min-w-0">
              <p class="text-sm font-semibold text-foreground">
                {scanStore.isRefreshingAfterClean ? 'Checking storage after cleanup' : 'Checking storage'}
              </p>
              <p class="mt-1 text-body text-muted-foreground">
                {scanStore.isRefreshingAfterClean
                  ? 'Cleanup finished. A new scan is checking what remains before results appear.'
                  : 'Scanning known caches to prepare a current inventory for review.'}
              </p>
            </div>
          </div>
          <Button
            variant="outline"
            size="sm"
            disabled={scanStore.isCancelling}
            onclick={() => void scanStore.cancelScan()}
            ariaLabel={scanStore.isCancelling ? 'Stopping scan' : 'Stop scan'}
            class="gap-1.5 shrink-0"
          >
            <Square size={12} />
            <span>{scanStore.isCancelling ? 'Stopping…' : 'Stop scan'}</span>
          </Button>
        </div>
        <div class="grid gap-3 rounded-lg bg-secondary/55 px-3 py-2 text-meta text-muted-foreground sm:grid-cols-2">
          <p class="min-w-0">Current area <span class="ml-1 font-medium text-foreground">{scanStore.currentRoot?.name ?? 'Preparing scan…'}</span></p>
          <p class="font-mono tabular-nums">{scanStore.foundItemCount} {scanStore.foundItemCount === 1 ? 'item' : 'items'} found so far</p>
        </div>
        {#if scanStore.currentRoot}
          <details class="text-caption text-muted-foreground">
            <summary class="w-fit cursor-pointer rounded-sm underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">Show current path</summary>
            <code class="mt-1 block break-all font-mono">{scanStore.currentRoot.path}</code>
          </details>
        {/if}
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
      <InlineNotice
        variant="error"
        title="Storage check needs attention"
        message={scanStore.error}
        actionLabel={!scanStore.isScanning && !scanStore.isCleaning ? 'Scan Again' : undefined}
        onAction={() => void scanStore.runScan()}
      />
    {/if}

    <!-- Scan freshness / remediation notice -->
    {#if !scanStore.isScanning && !scanStore.isCleaning && scan && scanStore.freshness !== 'fresh' && scanStore.freshness !== 'failed'}
      <ScanFreshnessNotice compact />
    {/if}

    <!-- Categories Section -->
    {#if !scanStore.isScanning && !scanStore.isCleaning && !scanStore.isRefreshingAfterClean}
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
          <HardDrive size={24} class="mx-auto mb-3 text-muted-foreground" aria-hidden="true" />
          <p class="text-sm font-medium text-foreground">Start with a storage scan</p>
          <p class="text-body text-muted-foreground">Find known caches, then choose what to review.</p>
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
