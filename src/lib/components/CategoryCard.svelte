<script lang="ts">
  import type { CategoryResult } from '../models/types';
  import { formatBytes } from '../utils/format';
  import { scanStore } from '../stores/scan.svelte';
  import Card from './Card.svelte';
  import Badge from './Badge.svelte';
  import Checkbox from './Checkbox.svelte';
  import {
    Bot,
    Code2,
    Container,
    Cpu,
    Boxes,
    ChevronRight,
  } from 'lucide-svelte';

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

  let cleanableItems = $derived(categoryResult.items.filter((i) => i.risk !== 'manual'));

  let allSelected = $derived.by(() => {
    if (cleanableItems.length === 0) return false;
    return cleanableItems.every((i) => scanStore.selectedMap[i.id]);
  });

  let selectedBytes = $derived.by(() => {
    return cleanableItems.reduce((acc, i) => {
      return scanStore.selectedMap[i.id]
        ? acc + (i.size.allocated ?? i.size.logical)
        : acc;
    }, 0);
  });

  let showSelectedBytes = $derived(
    selectedBytes > 0 &&
      selectedBytes !== categoryResult.total_bytes &&
      selectedBytes !== categoryResult.safe_bytes
  );

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
    class="flex min-w-0 flex-1 items-center justify-between gap-3"
    onclick={() => onSelectCategory?.(categoryResult)}
  >
    <div class="flex min-w-0 flex-1 items-center gap-3">
      <!-- Custom Checkbox (only for cleanable categories) -->
      {#if cleanableItems.length > 0}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div onclick={(e) => e.stopPropagation()}>
          <Checkbox
            checked={allSelected}
            onchange={handleToggleCheckbox}
            ariaLabel={`Select all ${categoryResult.display_name} items`}
          />
        </div>
      {:else}
        <div
          class="h-4 w-4 rounded border border-border/40 bg-secondary/30 flex items-center justify-center text-micro text-muted-foreground"
          title="Manual category: stateful resources are managed in dedicated adapter"
        >
          -
        </div>
      {/if}

      <div
        class="h-9 w-9 rounded-lg bg-secondary flex items-center justify-center text-foreground group-hover:bg-secondary/80 transition-colors"
      >
        <Icon size={18} />
      </div>

      <div class="min-w-0 flex-1">
        <div class="flex min-w-0 items-center">
          <h3 class="min-w-0 truncate text-sm font-medium text-foreground tracking-tight">
            {categoryResult.display_name}
          </h3>
        </div>
        <div class="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5">
          <span class="shrink-0 whitespace-nowrap text-xs text-muted-foreground font-mono">
            {categoryResult.items.length} items
          </span>
          {#if categoryResult.safe_bytes > 0}
            <span class="shrink-0 whitespace-nowrap text-meta text-success font-mono">
              Safe: {formatBytes(categoryResult.safe_bytes)}
            </span>
          {/if}
          {#if categoryResult.rebuild_bytes > 0}
            <span class="shrink-0 whitespace-nowrap text-meta text-warning font-mono">
              • Rebuild: {formatBytes(categoryResult.rebuild_bytes)}
            </span>
          {/if}
          {#if categoryResult.manual_bytes > 0}
            <span class="shrink-0 whitespace-nowrap text-meta text-destructive font-mono">
              • Manual: {formatBytes(categoryResult.manual_bytes)}
            </span>
          {/if}
        </div>
      </div>
    </div>

    <div class="flex shrink-0 items-center gap-3">
      <div class="w-[7rem] shrink-0 text-right">
        <span class="block whitespace-nowrap text-sm font-semibold font-mono text-foreground">
          {formatBytes(categoryResult.total_bytes)}
        </span>
        <span class="block whitespace-nowrap text-micro text-muted-foreground">Detected</span>
        {#if showSelectedBytes}
          <div class="whitespace-nowrap text-caption text-muted-foreground font-mono">
            Selected: {formatBytes(selectedBytes)}
          </div>
        {/if}
      </div>

      <ChevronRight
        size={16}
        class="shrink-0 text-muted-foreground group-hover:translate-x-0.5 transition-transform"
      />
    </div>
  </div>
</Card>
