<script lang="ts">
  import type { CategoryResult } from '../models/types';
  import { formatBytes } from '../utils/format';
  import { isActionable, presentedItems, summarizeCategory } from '../utils/cleanup';
  import { scanStore } from '../stores/scan.svelte';
  import Card from './Card.svelte';
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

  let allSelected = $derived.by(() => {
    if (cleanableItems.length === 0) return false;
    return cleanableItems.every((i) => scanStore.selectedMap[i.id]);
  });

  function handleToggleCheckbox(checked: boolean) {
    if (cleanableItems.length === 0) return;
    scanStore.toggleCategory(categoryResult.category, checked);
  }
</script>

<Card
  class="group cursor-pointer hover:border-primary/50 hover:bg-card/90 transition-colors duration-150 relative overflow-hidden"
>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    data-category-card-layout="stable"
    class="flex min-w-0 items-center gap-3"
    onclick={() => onSelectCategory?.(categoryResult)}
  >
    <!-- Custom Checkbox (only for cleanable categories) -->
    {#if cleanableItems.length > 0}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="shrink-0" onclick={(e) => e.stopPropagation()}>
        <Checkbox
          checked={allSelected}
          disabled={!scanStore.canClean}
          onchange={handleToggleCheckbox}
          ariaLabel={`Select all ${categoryResult.display_name} items`}
        />
      </div>
    {:else}
      <div
        class="shrink-0 h-4 w-4 rounded border border-border/40 bg-secondary/30 flex items-center justify-center text-micro text-muted-foreground"
      >
        -
      </div>
    {/if}

    <div
      class="shrink-0 h-9 w-9 rounded-lg bg-secondary flex items-center justify-center text-foreground group-hover:bg-secondary/80 transition-colors"
    >
      <Icon size={18} />
    </div>

    <div data-region="identity" class="min-w-0 flex-1">
      <h3 class="text-sm font-medium leading-5 text-foreground tracking-tight break-normal [overflow-wrap:normal]">
        {categoryResult.display_name}
      </h3>
      <p data-region="metadata" class="mt-0.5 text-xs text-muted-foreground">
        {presented.length} {presented.length === 1 ? 'item' : 'items'}
        {#if summary.cleanable_bytes === 0} · Nothing to clean{/if}
      </p>
    </div>

    <div
      data-region="metrics"
      class="shrink-0 text-right"
    >
      <span class="block whitespace-nowrap text-sm font-semibold font-mono tabular-nums text-foreground">
        {#if summary.cleanable_bytes > 0}{formatBytes(summary.cleanable_bytes)}{/if}
      </span>
      <span class="block whitespace-nowrap text-micro text-muted-foreground">
        {summary.cleanable_bytes > 0 ? 'Can clean' : ''}
      </span>
    </div>

    <ChevronRight
      size={16}
      class="shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5"
    />
  </div>
</Card>
