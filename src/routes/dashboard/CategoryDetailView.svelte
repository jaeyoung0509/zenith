<script lang="ts">
  import type { CategoryResult, RiskTier } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { formatBytes } from '../../lib/utils/format';
  import {
    filterAndSortCleanupItems,
    reclaimableBytes,
    type CleanupSortMode,
  } from '../../lib/utils/cleanup';
  import Button from '../../lib/components/Button.svelte';
  import ItemRow from '../../lib/components/ItemRow.svelte';
  import CleanResultModal from '../../lib/components/CleanResultModal.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import ProgressBar from '../../lib/components/ProgressBar.svelte';
  import Card from '../../lib/components/Card.svelte';
  import {
    ArrowLeft,
    Search,
    CheckSquare,
    Square,
    Info,
    Trash2,
    Boxes,
    Container,
    AlertCircle,
  } from 'lucide-svelte';

  interface Props {
    categoryResult: CategoryResult;
    onBack: () => void;
    onNavigateTab?: (tab: 'models' | 'docker') => void;
  }

  let { categoryResult, onBack, onNavigateTab }: Props = $props();

  let searchQuery = $state('');
  let selectedRiskFilter = $state<RiskTier | 'all'>('all');
  let sortMode = $state<CleanupSortMode>('size');
  let showResultModal = $state(false);

  let filteredItems = $derived.by(() => {
    return filterAndSortCleanupItems(
      categoryResult.items,
      selectedRiskFilter,
      searchQuery,
      sortMode
    );
  });

  let cleanableFilteredItems = $derived(filteredItems.filter((i) => i.risk !== 'manual'));

  let allFilteredSelected = $derived.by(() => {
    if (cleanableFilteredItems.length === 0) return false;
    return cleanableFilteredItems.every((i) => scanStore.selectedMap[i.id]);
  });

  let categorySelectedBytes = $derived.by(() =>
    categoryResult.items.reduce(
      (total, item) =>
        total + (item.risk !== 'manual' && scanStore.selectedMap[item.id] ? reclaimableBytes(item) : 0),
      0
    )
  );

  function toggleAllFiltered() {
    if (cleanableFilteredItems.length === 0) return;
    const next = !allFilteredSelected;
    for (const item of cleanableFilteredItems) {
      scanStore.setItemSelected(item.id, next);
    }
  }

  function cleanSelected() {
    scanStore.cleanItems(categoryResult.items).then((result) => {
      if (result) showResultModal = true;
    });
  }
</script>

<div class="space-y-5">
  <!-- Back Button & Category Header -->
  <div class="flex items-center justify-between pb-2 border-b border-border/60">
    <div class="flex items-center gap-3">
      <Button variant="ghost" size="icon" onclick={onBack} class="h-8 w-8">
        <ArrowLeft size={16} />
      </Button>
      <div>
        <h2 class="text-base font-semibold text-foreground tracking-tight">
          {categoryResult.display_name}
        </h2>
        <p class="text-xs text-muted-foreground">
          {filteredItems.length} detected {filteredItems.length === 1 ? 'location' : 'locations'} • {formatBytes(categoryResult.total_bytes)} detected
        </p>
      </div>
    </div>

    <div class="flex items-center gap-2">
      <Button
        variant="outline"
        size="sm"
        disabled={cleanableFilteredItems.length === 0}
        onclick={toggleAllFiltered}
        class="gap-1.5 text-xs"
      >
        {#if allFilteredSelected}
          <Square size={13} />
          <span>Deselect Filtered</span>
        {:else}
          <CheckSquare size={13} class="text-success" />
          <span>Select Filtered</span>
        {/if}
      </Button>

      <Button
        variant="primary"
        size="sm"
        class="gap-1.5 min-w-[90px]"
        disabled={categorySelectedBytes === 0 || scanStore.isCleaning}
        onclick={cleanSelected}
      >
        {#if scanStore.isCleaning}
          <DeletingDots size="xs" />
          <span>Cleaning…</span>
        {:else}
          <Trash2 size={13} />
          <span>Clean {formatBytes(categorySelectedBytes)}</span>
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
        placeholder="Filter by name or path..."
        class="w-full h-8 pl-8 pr-3 text-xs rounded-lg border border-border bg-card text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
      />
    </div>

    <!-- Risk Filter Tabs -->
    <div class="flex items-center gap-2 self-stretch sm:self-auto">
      <select
        bind:value={sortMode}
        aria-label="Sort cleanup items"
        class="h-8 rounded-lg border border-border bg-card px-2.5 text-xs text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
      >
        <option value="size">Largest first</option>
        <option value="modified">Recently modified</option>
        <option value="name">Name A–Z</option>
      </select>

      <div class="flex items-center gap-1 bg-secondary/60 p-1 rounded-lg">
      <button
        type="button"
        onclick={() => (selectedRiskFilter = 'all')}
        class="px-2.5 py-1 text-xs font-medium rounded-md transition-colors {selectedRiskFilter ===
        'all'
          ? 'bg-background text-foreground shadow-sm'
          : 'text-muted-foreground hover:text-foreground'}"
      >
        All ({categoryResult.items.length})
      </button>
      <button
        type="button"
        onclick={() => (selectedRiskFilter = 'safe')}
        class="px-2.5 py-1 text-xs font-medium rounded-md transition-colors {selectedRiskFilter ===
        'safe'
          ? 'bg-background text-success shadow-sm'
          : 'text-muted-foreground hover:text-foreground'}"
      >
        Safe ({categoryResult.items.filter((i) => i.risk === 'safe').length})
      </button>
      <button
        type="button"
        onclick={() => (selectedRiskFilter = 'rebuild')}
        class="px-2.5 py-1 text-xs font-medium rounded-md transition-colors {selectedRiskFilter ===
        'rebuild'
          ? 'bg-background text-warning shadow-sm'
          : 'text-muted-foreground hover:text-foreground'}"
      >
        Rebuild ({categoryResult.items.filter((i) => i.risk === 'rebuild').length})
      </button>
      <button
        type="button"
        onclick={() => (selectedRiskFilter = 'manual')}
        class="px-2.5 py-1 text-xs font-medium rounded-md transition-colors {selectedRiskFilter ===
        'manual'
          ? 'bg-background text-destructive shadow-sm'
          : 'text-muted-foreground hover:text-foreground'}"
      >
        Manual ({categoryResult.items.filter((i) => i.risk === 'manual').length})
      </button>
      </div>
    </div>
  </div>

  {#if selectedRiskFilter === 'all' || selectedRiskFilter === 'rebuild'}
    <div class="flex items-start gap-2.5 rounded-xl border border-warning/20 bg-warning/5 px-4 py-3">
      <Info size={15} class="mt-0.5 shrink-0 text-warning" />
      <p class="text-meta leading-relaxed text-muted-foreground">
        <span class="font-medium text-warning">Rebuild</span> items are safe to remove, but dependencies or indexes will be downloaded or rebuilt the next time you use that tool. They stay unselected until you choose them.
      </p>
    </div>
  {/if}

  <!-- Items List -->
  {#if filteredItems.length > 0}
    <div class="space-y-2">
      {#each filteredItems as item (item.id)}
        <ItemRow {item} />
      {/each}
    </div>
  {:else}
    <div class="py-16 text-center text-xs text-muted-foreground">
      No items match your search or risk filter.
    </div>
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
