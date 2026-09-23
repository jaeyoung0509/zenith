<script lang="ts">
  import type { CategoryResult, PlanPreview } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { formatBytes } from '../../lib/utils/format';
  import {
    cleanableBytes,
    emptyCategoryMessage,
    filterAndSortCleanupItems,
    isActionable,
    summarizeCategory,
    type CleanupSortMode,
  } from '../../lib/utils/cleanup';
  import Button from '../../lib/components/Button.svelte';
  import ItemRow from '../../lib/components/ItemRow.svelte';
  import CleanResultModal from '../../lib/components/CleanResultModal.svelte';
  import CleanupReviewDialog from '../../lib/components/CleanupReviewDialog.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import ProgressBar from '../../lib/components/ProgressBar.svelte';
  import Card from '../../lib/components/Card.svelte';
  import ScanFreshnessNotice from '../../lib/components/ScanFreshnessNotice.svelte';
  import {
    ArrowLeft,
    Search,
    CheckSquare,
    Square,
    Trash2,
    Boxes,
    Container,
    AlertCircle,
  } from '@lucide/svelte';

  interface Props {
    categoryResult: CategoryResult;
    onBack: () => void;
    onNavigateTab?: (tab: 'models' | 'docker') => void;
  }

  let { categoryResult, onBack, onNavigateTab }: Props = $props();

  let searchQuery = $state('');
  let sortMode = $state<CleanupSortMode>('size');
  let showResultModal = $state(false);
  let review = $state<{ scanId: string; plan: PlanPreview } | null>(null);
  let isPreparingReview = $state(false);

  let filteredItems = $derived.by(() => {
    return filterAndSortCleanupItems(
      categoryResult.items,
      'all',
      searchQuery,
      sortMode
    );
  });

  let cleanableFilteredItems = $derived(filteredItems.filter(isActionable));

  let noCleanableReason = $derived.by(() => {
    if (filteredItems.length === 0) return 'No matching items.';
    if (cleanableFilteredItems.length > 0) return null;
    return emptyCategoryMessage(
      summarizeCategory(filteredItems, scanStore.selectedMap),
      categoryResult.quality,
      categoryResult.category
    );
  });
  let selectFilteredTitle = $derived.by(() => {
    if (!scanStore.canClean) return 'Run a fresh scan before changing the selection.';
    return noCleanableReason ?? undefined;
  });

  let allFilteredSelected = $derived.by(() => {
    if (cleanableFilteredItems.length === 0) return false;
    return cleanableFilteredItems.every((i) => scanStore.selectedMap[i.id]);
  });

  let summary = $derived(summarizeCategory(categoryResult.items, scanStore.selectedMap));
  let presentedCount = $derived(summary.visible_count);

  let categorySelectedBytes = $derived(summary.selected_bytes);

  function toggleAllFiltered() {
    if (cleanableFilteredItems.length === 0) return;
    const next = !allFilteredSelected;
    for (const item of cleanableFilteredItems) {
      scanStore.setItemSelected(item.id, next);
    }
  }

  async function cleanSelected() {
    const scan = scanStore.lastScan;
    if (!scan || !scanStore.canClean) return;
    const items = categoryResult.items.filter(
      (item) => scanStore.selectedMap[item.id] && isActionable(item)
    );
    if (items.length === 0) return;
    const scanId = scan.scan_id;
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
    if (!review || review.scanId !== scanStore.lastScan?.scan_id || !scanStore.canClean) return;
    const plan = review.plan;
    review = null;
    scanStore.executePreparedPlan(plan, true).then((result) => {
      if (result) showResultModal = true;
    });
  }
</script>

<div class="space-y-5">
  <ScanFreshnessNotice />
  <!-- Back Button & Category Header -->
  <div class="flex items-center justify-between pb-2 border-b border-border/60">
    <div class="flex items-center gap-3">
      <Button variant="ghost" size="icon" onclick={onBack} class="h-8 w-8">
        <ArrowLeft size={16} />
      </Button>
      <div>
        <div class="flex items-center gap-2">
          <h2 class="text-base font-semibold text-foreground tracking-tight">
            {categoryResult.display_name}
          </h2>
        </div>
        <p class="text-xs text-muted-foreground">
          {presentedCount} {presentedCount === 1 ? 'item' : 'items'} · {summary.cleanable_bytes > 0 ? `${formatBytes(summary.cleanable_bytes)} can be cleaned` : summary.cleanable_count > 0 ? 'Amount varies by owner' : emptyCategoryMessage(summary, categoryResult.quality, categoryResult.category)}
        </p>
      </div>
    </div>

    <div class="flex items-center gap-2">
      <Button
        variant="outline"
        size="sm"
        disabled={cleanableFilteredItems.length === 0 || !scanStore.canClean}
        title={selectFilteredTitle}
        onclick={toggleAllFiltered}
        class="gap-1.5 text-xs"
      >
        {#if allFilteredSelected}
          <Square size={13} />
          <span>Deselect</span>
        {:else}
          <CheckSquare size={13} class="text-success" />
          <span>Select all</span>
        {/if}
      </Button>

      <Button
        variant="primary"
        size="sm"
        class="gap-1.5 min-w-[90px]"
        disabled={summary.selected_count === 0 || !scanStore.canClean || isPreparingReview}
        onclick={cleanSelected}
      >
        {#if scanStore.isCleaning || isPreparingReview}
          <DeletingDots size="xs" />
          <span>{isPreparingReview ? 'Reviewing…' : 'Cleaning…'}</span>
        {:else}
          <Trash2 size={13} />
          <span>{categorySelectedBytes > 0 ? `Clean ${formatBytes(categorySelectedBytes)}` : 'Run owner cleanup'}</span>
        {/if}
      </Button>
    </div>
  </div>

  <!-- Adapter Quick Link Banners for Stateful Categories -->
  {#if categoryResult.category === 'model'}
    <div class="flex items-center justify-between p-3.5 rounded-xl border border-warning/25 bg-warning/10 text-warning text-xs">
      <div class="flex items-center gap-2.5">
        <Boxes size={16} class="text-warning shrink-0" />
        <span>Local models are stateful assets. Manage, inspect, or delete them safely in the Local Models manager.</span>
      </div>
      {#if onNavigateTab}
        <Button
          variant="outline"
          size="sm"
          class="border-warning/40 text-warning hover:bg-warning/20 text-xs shrink-0"
          onclick={() => onNavigateTab('models')}
        >
          <span>Open Local Models →</span>
        </Button>
      {/if}
    </div>
  {:else if categoryResult.category === 'container'}
    <div class="flex items-center justify-between p-3.5 rounded-xl border border-cyan-500/25 bg-cyan-500/10 text-cyan-300 text-xs">
      <div class="flex items-center gap-2.5">
        <Container size={16} class="text-cyan-400 shrink-0" />
        <span>Docker and OrbStack data are stateful resources. Zenith reports their storage without deleting it; use the owning container manager for changes.</span>
      </div>
      {#if onNavigateTab}
        <Button
          variant="outline"
          size="sm"
          class="border-cyan-500/40 text-cyan-300 hover:bg-cyan-500/20 text-xs shrink-0"
          onclick={() => onNavigateTab('docker')}
        >
          <span>Open Containers →</span>
        </Button>
      {/if}
    </div>
  {/if}

  <!-- Cleaning In Progress Bar -->
  {#if scanStore.isCleaning}
    <Card class="p-3.5 bg-secondary/60 border-primary/40 shadow-sm transition-all duration-200">
      <div class="space-y-1.5">
        <div class="flex items-center justify-between text-xs">
          <span class="font-medium text-foreground flex items-center gap-2">
            <DeletingDots size="xs" />
            <span>Cleaning: {scanStore.cleanProgress.currentItem}</span>
          </span>
          <span class="font-mono text-muted-foreground font-semibold">
            {scanStore.cleanProgress.percent}%
          </span>
        </div>
        <ProgressBar value={scanStore.cleanProgress.percent} height="h-2" color="bg-primary" animated={true} />
      </div>
    </Card>
  {/if}

  <!-- Error Alert -->
  {#if scanStore.error}
    <div class="p-3 rounded-xl bg-destructive/15 border border-destructive/30 text-destructive flex items-center gap-2.5 text-xs">
      <AlertCircle size={15} class="shrink-0" />
      <span>{scanStore.error}</span>
    </div>
  {/if}

  <!-- Filter & Search Toolbar -->
  <div class="flex flex-col sm:flex-row items-center justify-between gap-3">
    <!-- Search Box -->
    <div class="relative w-full sm:w-72">
      <Search
        size={14}
        class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground pointer-events-none"
      />
      <input
        type="text"
        bind:value={searchQuery}
        aria-label="Filter cleanup items by name or path"
        placeholder="Filter by name or path..."
        class="w-full h-8 pl-8 pr-3 text-xs rounded-lg border border-border bg-card text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
      />
    </div>

    <div class="flex flex-col items-stretch gap-1 sm:items-end sm:self-auto">
      <div class="flex items-center gap-2">
        <select
          bind:value={sortMode}
          aria-label="Sort cleanup items"
          class="h-8 rounded-lg border border-border bg-card px-2.5 text-xs text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
        >
          <option value="size">Largest first</option>
          <option value="modified">Recently modified</option>
          <option value="name">Name A–Z</option>
        </select>

      </div>

      {#if noCleanableReason}
        <p class="text-caption text-muted-foreground">{noCleanableReason}</p>
      {/if}
    </div>
  </div>


  <!-- Items List -->
  {#if filteredItems.length > 0}
    <div class="space-y-2">
      {#each filteredItems as item (item.id)}
        <ItemRow {item} />
      {/each}
    </div>
  {:else}
    <div class="py-16 text-center text-xs text-muted-foreground">
      No items match your search.
    </div>
  {/if}

  {#if review}
    <CleanupReviewDialog
      plan={review.plan}
      disabled={review.scanId !== scanStore.lastScan?.scan_id || !scanStore.canClean}
      onCancel={() => (review = null)}
      onConfirm={confirmCleanup}
    />
  {/if}

  {#if showResultModal && scanStore.lastCleanResult}
    <CleanResultModal
      result={scanStore.lastCleanResult}
      onClose={() => {
        showResultModal = false;
        onBack();
      }}
    />
  {/if}
</div>
