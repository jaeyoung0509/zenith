<script lang="ts">
  import type { CategoryResult } from '../models/types';
  import { formatBytes } from '../utils/format';
  import { scanStore } from '../stores/scan.svelte';
  import Card from './Card.svelte';
  import Badge from './Badge.svelte';
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

  function handleToggleCheckbox(e: MouseEvent) {
    e.stopPropagation();
    if (cleanableItems.length === 0) return;
    scanStore.toggleCategory(categoryResult.category, !allSelected);
  }
</script>

<Card
  class="group cursor-pointer hover:border-primary/40 transition-all duration-150 relative overflow-hidden"
>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="flex items-center justify-between"
    onclick={() => onSelectCategory?.(categoryResult)}
  >
    <div class="flex items-center space-x-3">
      <!-- Custom Checkbox (only for cleanable categories) -->
      {#if cleanableItems.length > 0}
        <input
          type="checkbox"
          checked={allSelected}
          onclick={handleToggleCheckbox}
          class="h-4 w-4 rounded border-border text-primary focus:ring-ring transition-colors cursor-pointer accent-primary"
        />
      {:else}
        <div
          class="h-4 w-4 rounded border border-border/40 bg-secondary/30 flex items-center justify-center text-[9px] text-muted-foreground"
          title="Manual category: stateful resources are managed in dedicated adapter"
        >
          -
        </div>
      {/if}

      <div
        class="h-9 w-9 rounded-lg bg-secondary flex items-center justify-center text-foreground group-hover:scale-105 transition-transform"
      >
        <Icon size={18} />
      </div>

      <div>
        <div class="flex items-center gap-2">
          <h3 class="text-sm font-medium text-foreground tracking-tight">
            {categoryResult.display_name}
          </h3>
          <span class="text-xs text-muted-foreground font-mono">
            {categoryResult.items.length} items
          </span>
        </div>
        <div class="flex items-center gap-1.5 mt-0.5">
          {#if categoryResult.safe_bytes > 0}
            <span class="text-[11px] text-emerald-500 font-mono">
              Safe: {formatBytes(categoryResult.safe_bytes)}
            </span>
          {/if}
          {#if categoryResult.rebuild_bytes > 0}
            <span class="text-[11px] text-amber-500 font-mono">
              • Rebuild: {formatBytes(categoryResult.rebuild_bytes)}
            </span>
          {/if}
          {#if categoryResult.manual_bytes > 0}
            <span class="text-[11px] text-rose-500 font-mono">
              • Manual: {formatBytes(categoryResult.manual_bytes)}
            </span>
          {/if}
        </div>
      </div>
    </div>

    <div class="flex items-center gap-3">
      <div class="text-right">
        <span class="text-sm font-semibold font-mono text-foreground">
          {formatBytes(categoryResult.total_bytes)}
        </span>
        {#if selectedBytes > 0 && selectedBytes !== categoryResult.total_bytes}
          <div class="text-[10px] text-muted-foreground font-mono">
            Selected: {formatBytes(selectedBytes)}
          </div>
        {/if}
      </div>

      <ChevronRight
        size={16}
        class="text-muted-foreground group-hover:translate-x-0.5 transition-transform"
      />
    </div>
  </div>
</Card>
