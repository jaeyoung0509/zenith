<script lang="ts">
  import type { CategoryResult, RiskTier } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { formatBytes } from '../../lib/utils/format';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import ItemRow from '../../lib/components/ItemRow.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import {
    ArrowLeft,
    Search,
    CheckSquare,
    Square,
    Filter,
  } from 'lucide-svelte';

  interface Props {
    categoryResult: CategoryResult;
    onBack: () => void;
  }

  let { categoryResult, onBack }: Props = $props();

  let searchQuery = $state('');
  let selectedRiskFilter = $state<RiskTier | 'all'>('all');

  let filteredItems = $derived.by(() => {
    return categoryResult.items.filter((item) => {
      // Risk filter
      if (selectedRiskFilter !== 'all' && item.risk !== selectedRiskFilter) {
        return false;
      }
      // Search query
      if (searchQuery.trim()) {
        const q = searchQuery.toLowerCase();
        return (
          item.name.toLowerCase().includes(q) ||
          item.path.toLowerCase().includes(q) ||
          item.description.toLowerCase().includes(q)
        );
      }
      return true;
    });
  });

  let allFilteredSelected = $derived.by(() => {
    if (filteredItems.length === 0) return false;
    return filteredItems.every((i) => scanStore.selectedMap[i.id]);
  });

  function toggleAllFiltered() {
    const next = !allFilteredSelected;
    for (const item of filteredItems) {
      scanStore.setItemSelected(item.id, next);
    }
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
          {categoryResult.items.length} detected locations • {formatBytes(categoryResult.total_bytes)} total
        </p>
      </div>
    </div>

    <div class="flex items-center gap-2">
      <Button
        variant="outline"
        size="sm"
        onclick={toggleAllFiltered}
        class="gap-1.5 text-xs"
      >
        {#if allFilteredSelected}
          <Square size={13} />
          <span>Deselect Filtered</span>
        {:else}
          <CheckSquare size={13} class="text-emerald-500" />
          <span>Select Filtered</span>
        {/if}
      </Button>
    </div>
  </div>

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
    <div class="flex items-center gap-1 bg-secondary/60 p-1 rounded-lg self-stretch sm:self-auto">
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
          ? 'bg-background text-emerald-500 shadow-sm'
          : 'text-muted-foreground hover:text-foreground'}"
      >
        Safe ({categoryResult.items.filter((i) => i.risk === 'safe').length})
      </button>
      <button
        type="button"
        onclick={() => (selectedRiskFilter = 'rebuild')}
        class="px-2.5 py-1 text-xs font-medium rounded-md transition-colors {selectedRiskFilter ===
        'rebuild'
          ? 'bg-background text-amber-500 shadow-sm'
          : 'text-muted-foreground hover:text-foreground'}"
      >
        Rebuild ({categoryResult.items.filter((i) => i.risk === 'rebuild').length})
      </button>
      <button
        type="button"
        onclick={() => (selectedRiskFilter = 'manual')}
        class="px-2.5 py-1 text-xs font-medium rounded-md transition-colors {selectedRiskFilter ===
        'manual'
          ? 'bg-background text-rose-500 shadow-sm'
          : 'text-muted-foreground hover:text-foreground'}"
      >
        Manual ({categoryResult.items.filter((i) => i.risk === 'manual').length})
      </button>
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
      No items match your search or risk filter.
    </div>
  {/if}
</div>
