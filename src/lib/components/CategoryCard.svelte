<script lang="ts">
  import type { CategoryResult, CleanupEligibility } from '../models/types';
  import { formatBytes } from '../utils/format';
  import { cleanableBytes, isCleanable, observedByteRange, presentedItems } from '../utils/cleanup';
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

  let cleanableItems = $derived(categoryResult.items.filter(isCleanable));
  let observedRange = $derived(
    observedByteRange(categoryResult.total_bytes, categoryResult.ambiguous_overlap_bytes ?? 0)
  );

  let allSelected = $derived.by(() => {
    if (cleanableItems.length === 0) return false;
    return cleanableItems.every((i) => scanStore.selectedMap[i.id]);
  });

  let selectedBytes = $derived.by(() => {
    return cleanableItems.reduce((acc, i) => {
      return scanStore.selectedMap[i.id] ? acc + cleanableBytes(i) : acc;
    }, 0);
  });

  let showSelectedBytes = $derived(
    selectedBytes > 0 &&
      selectedBytes !== categoryResult.total_bytes &&
      selectedBytes !== categoryResult.safe_bytes
  );

  // A scan can report bytes it will not remove: the age policy is not
  // satisfied yet, or an opt-in scope is off. The card names those states and
  // their amounts so a detected total is never left unexplained.
  const notCleanableStates: { eligibility: CleanupEligibility; label: string; explanation: string }[] = [
    {
      eligibility: 'recent',
      label: 'Recently used',
      explanation: 'Discovered, but not old enough for the age policy: counted, not cleanable yet.',
    },
    {
      eligibility: 'policy_gated',
      label: 'Outside the current scope',
      explanation: 'Discovered, but the current settings do not clean it: an opt-in scope is off.',
    },
  ];

  let notCleanable = $derived.by(() => {
    const buckets = categoryResult.eligibility?.buckets ?? [];
    return notCleanableStates
      .map((state) => ({
        ...state,
        bytes: buckets.reduce(
          (sum, bucket) =>
            bucket.eligibility === state.eligibility
              ? sum + Math.max(0, bucket.observed_bytes - bucket.cleanable_bytes)
              : sum,
          0
        ),
      }))
      .filter((state) => state.bytes > 0);
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
    class="grid min-w-0 flex-1 grid-cols-[auto_auto_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1.5 sm:grid-cols-[auto_auto_minmax(0,1fr)_minmax(9rem,auto)_auto]"
    onclick={() => onSelectCategory?.(categoryResult)}
  >
    <!-- Custom Checkbox (only for cleanable categories) -->
    {#if cleanableItems.length > 0}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="col-start-1 row-start-1" onclick={(e) => e.stopPropagation()}>
        <Checkbox
          checked={allSelected}
          disabled={!scanStore.canClean}
          onchange={handleToggleCheckbox}
          ariaLabel={`Select all ${categoryResult.display_name} items`}
        />
      </div>
    {:else}
      <div
        class="col-start-1 row-start-1 h-4 w-4 rounded border border-border/40 bg-secondary/30 flex items-center justify-center text-micro text-muted-foreground"
        title="Manual category: stateful resources are managed in dedicated adapter"
      >
        -
      </div>
    {/if}

    <div
      class="col-start-2 row-start-1 h-9 w-9 rounded-lg bg-secondary flex items-center justify-center text-foreground group-hover:bg-secondary/80 transition-colors"
    >
      <Icon size={18} />
    </div>

    <div data-region="identity" class="col-start-3 row-start-1 min-w-0 self-center">
      <div class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
        <h3 class="min-w-0 break-words text-sm font-medium leading-5 text-foreground tracking-tight">
          {categoryResult.display_name}
        </h3>
        {#if categoryResult.quality === 'partial'}
          <span class="shrink-0 px-1.5 py-0.5 rounded text-micro font-medium border border-warning/40 text-warning bg-warning/10" title="Some paths could not be fully inspected">
            Partial
          </span>
        {/if}
      </div>
      <div data-region="metadata" class="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5">
        <span class="shrink-0 whitespace-nowrap text-xs text-muted-foreground font-mono">
          {presented.length} items
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
        {#if (categoryResult.skipped_entry_count ?? 0) > 0}
          <span
            class="shrink-0 whitespace-nowrap text-meta text-muted-foreground font-mono"
            title="Excluded, protected, or unreadable entries the measurement did not count"
          >
            • {categoryResult.skipped_entry_count} entries skipped
          </span>
        {/if}
        {#each notCleanable as state (state.eligibility)}
          <span
            class="shrink-0 whitespace-nowrap text-meta text-muted-foreground font-mono"
            title={state.explanation}
          >
            • {state.label}: {formatBytes(state.bytes)}
          </span>
        {/each}
        {#if (categoryResult.incomplete_item_count ?? 0) > 0}
          <span
            class="shrink-0 whitespace-nowrap text-meta text-muted-foreground font-mono"
            title="Items whose locations could not be fully inspected: partly measured or inaccessible"
          >
            • {categoryResult.incomplete_item_count} items not fully measured
          </span>
        {/if}
      </div>
    </div>

    <div
      data-region="metrics"
      class="col-start-3 row-start-2 min-w-0 text-left sm:col-start-4 sm:row-start-1 sm:min-w-[9rem] sm:text-right"
    >
      <span class="block whitespace-nowrap text-sm font-semibold font-mono tabular-nums text-foreground">
        {#if observedRange.isAmbiguous}
          {formatBytes(observedRange.lower)} – {formatBytes(observedRange.upper)}
        {:else}
          {categoryResult.quality === 'partial' ? '≥ ' : ''}{formatBytes(categoryResult.total_bytes)}
        {/if}
      </span>
      <span class="block whitespace-nowrap text-micro text-muted-foreground">
        {observedRange.isAmbiguous ? 'Observed range' : 'Detected'}
      </span>
      {#if showSelectedBytes}
        <span class="block whitespace-nowrap text-caption text-muted-foreground font-mono tabular-nums">
          Selected: {formatBytes(selectedBytes)}
        </span>
      {/if}
    </div>

    <ChevronRight
      size={16}
      class="col-start-4 row-start-1 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5 sm:col-start-5"
    />
  </div>
</Card>
