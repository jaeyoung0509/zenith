<script lang="ts">
  import type { CategoryResult } from '../models/types';
  import { formatBytes } from '../utils/format';
  import { emptyCategoryMessage, isActionable, presentedItems, summarizeCategory } from '../utils/cleanup';
  import { scanStore } from '../stores/scan.svelte';
  import Checkbox from './Checkbox.svelte';
  import {
    Bot,
    Code2,
    Container,
    Cpu,
    Boxes,
    ChevronRight,
  } from '@lucide/svelte';

  interface Props {
    categoryResult: CategoryResult;
    onSelectCategory?: (category: CategoryResult) => void;
  }

  let { categoryResult, onSelectCategory }: Props = $props();

  const icons = {
    ai: Bot,
    developer: Code2,
    container: Container,
    model: Boxes,
    system: Cpu,
  };

  let Icon = $derived(icons[categoryResult.category] || Boxes);

  // The card and its detail view read the same presented set, so a card can
  // never advertise a location the detail view omits (or vice versa).
  let presented = $derived(presentedItems(categoryResult.items));

  let cleanableItems = $derived(categoryResult.items.filter(isActionable));
  let summary = $derived(summarizeCategory(categoryResult.items, scanStore.selectedMap));
  let emptyMessage = $derived(emptyCategoryMessage(summary, categoryResult.quality, categoryResult.category));

  let allSelected = $derived.by(() => {
    if (cleanableItems.length === 0) return false;
    return cleanableItems.every((i) => scanStore.selectedMap[i.id]);
  });

  function handleToggleCheckbox(checked: boolean) {
    if (cleanableItems.length === 0) return;
    scanStore.toggleCategory(categoryResult.category, checked);
  }
</script>

<div class="category-row group transition-ui hover:bg-accent/40">
  <div
    data-category-card-layout="stable"
    class="flex min-w-0 items-center gap-3"
  >
    <!-- Custom Checkbox (only for cleanable categories) -->
    {#if cleanableItems.length > 0}
      <div class="shrink-0">
        <Checkbox
          class="h-8 w-8"
          checked={allSelected}
          disabled={!scanStore.canClean}
          onchange={handleToggleCheckbox}
          ariaLabel={`Select all ${categoryResult.display_name} items`}
        />
      </div>
    {:else}
      <div
        class="shrink-0 h-8 w-8 flex items-center justify-center text-meta text-muted-foreground"
        aria-label="No selectable items"
      >
        -
      </div>
    {/if}

    <button
      type="button"
      class="category-detail flex min-w-0 flex-1 items-center gap-3 rounded-lg text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring cursor-pointer"
      aria-label={`Open ${categoryResult.display_name} category`}
      onclick={() => onSelectCategory?.(categoryResult)}
    >
    <div
      class="shrink-0 h-9 w-9 rounded-lg flex items-center justify-center text-foreground transition-ui {allSelected
        ? 'bg-accent'
        : 'bg-secondary'}"
    >
      <Icon size={18} aria-hidden="true" />
    </div>

    <div data-region="identity" class="min-w-0 flex-1">
      <h3 class="text-body font-medium leading-5 text-foreground tracking-tight break-normal [overflow-wrap:normal]">
        {categoryResult.display_name}
      </h3>
      <p data-region="metadata" class="mt-0.5 text-meta text-muted-foreground">
        {presented.length} {presented.length === 1 ? 'item' : 'items'}
        {#if emptyMessage} · {emptyMessage}{/if}
      </p>
    </div>

    <div
      data-region="metrics"
      class="shrink-0 text-right category-amount"
    >
      <span class="block whitespace-nowrap text-body font-semibold font-mono tabular-nums text-foreground">
        {#if summary.cleanable_bytes > 0}
          {formatBytes(summary.cleanable_bytes)}
        {:else if summary.cleanable_count > 0}
          Amount varies
        {:else}
          —
        {/if}
      </span>
      <span class="block whitespace-nowrap text-caption text-muted-foreground">
        {summary.cleanable_bytes > 0 ? 'Can clean' : summary.cleanable_count > 0 ? 'Owner decides' : ''}
      </span>
    </div>

    <ChevronRight
      size={16}
      class="shrink-0 text-muted-foreground"
      aria-hidden="true"
    />
    </button>
  </div>
</div>

<style>
  .category-row { padding: 8px 12px 8px 8px; }
  .category-row + :global(.category-row) { border-top: 1px solid hsl(var(--border)); }
  .category-detail { min-height: 44px; padding: 4px; scroll-margin-block: 16px 112px; }
  .category-amount { width: 112px; }
  @container (max-width: 560px) {
    .category-amount { width: 96px; }
  }
</style>
