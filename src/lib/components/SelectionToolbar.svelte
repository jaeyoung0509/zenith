<script lang="ts">
  import type { Snippet } from "svelte";
  import Button from "./Button.svelte";
  import ByteValue from "./ByteValue.svelte";
  import { CheckSquare, ListChecks, Square } from "@lucide/svelte";

  interface Props {
    selectedCount: number;
    totalCount?: number;
    selectedBytes?: number;
    safeBytes?: number;
    rebuildBytes?: number;
    manualBytes?: number;
    manualCount?: number;
    onSelectAll?: () => void;
    onSelectSafe?: () => void;
    onDeselectAll?: () => void;
    actionLabel?: string;
    onAction?: () => void;
    isActionDisabled?: boolean;
    isActionLoading?: boolean;
    isSelectionDisabled?: boolean;
    class?: string;
    extraActions?: Snippet;
  }

  let {
    selectedCount,
    totalCount,
    selectedBytes = 0,
    safeBytes = 0,
    rebuildBytes = 0,
    manualBytes = 0,
    manualCount = 0,
    onSelectAll,
    onSelectSafe,
    onDeselectAll,
    actionLabel = "Clean Selected",
    onAction,
    isActionDisabled = false,
    isActionLoading = false,
    isSelectionDisabled = false,
    class: className = "",
    extraActions,
  }: Props = $props();
</script>

<div
  role="group"
  aria-label="Cleanup selection and actions"
  class="flex flex-col sm:flex-row sm:items-center justify-between gap-3 rounded-xl border border-border bg-card p-3 {className}"
>
  <div class="flex flex-wrap items-center gap-2">
    {#if onSelectSafe}
      <Button
        variant="ghost"
        size="sm"
        disabled={isSelectionDisabled}
        onclick={onSelectSafe}
        class="text-meta text-muted-foreground hover:text-foreground"
      >
        <CheckSquare size={13} class="mr-1 text-success" />
        <span>Select recommended</span>
      </Button>
    {/if}

    {#if onSelectAll}
      <Button
        variant="ghost"
        size="sm"
        disabled={isSelectionDisabled}
        onclick={onSelectAll}
        class="text-meta text-muted-foreground hover:text-foreground"
      >
        <span>Select All</span>
      </Button>
    {/if}

    {#if onDeselectAll}
      <Button
        variant="ghost"
        size="sm"
        disabled={isSelectionDisabled || selectedCount === 0}
        onclick={onDeselectAll}
        class="text-meta text-muted-foreground hover:text-foreground"
      >
        <Square size={13} class="mr-1" />
        <span>Deselect</span>
      </Button>
    {/if}

    <div class="h-4 w-px bg-border mx-1 hidden sm:block"></div>

    <span class="text-meta text-muted-foreground">
      <span class="font-medium text-foreground">{selectedCount}</span>
      {#if totalCount !== undefined}
        <span> of {totalCount}</span>
      {/if}
      selected
      {#if selectedBytes > 0}
        · <span class="font-mono font-semibold text-foreground"><ByteValue bytes={selectedBytes} /></span>
      {:else if selectedCount > manualCount}
        · <span>Amount varies by owner</span>
      {/if}
    </span>

    {#if safeBytes > 0 || rebuildBytes > 0 || manualCount > 0}
      <span class="inline-flex items-center gap-1.5 text-meta font-mono">
        {#if safeBytes > 0}
          <span class="text-success">✓ <ByteValue bytes={safeBytes} /> Safe</span>
        {/if}
        {#if rebuildBytes > 0}
          <span class="text-warning">↻ <ByteValue bytes={rebuildBytes} /> Rebuildable</span>
        {/if}
        {#if manualCount > 0}
          <span class="text-destructive">
            {manualCount} {manualCount === 1 ? 'item needs' : 'items need'} a separate action{#if manualBytes > 0} · <ByteValue bytes={manualBytes} />{/if}
          </span>
        {/if}
      </span>
    {/if}
  </div>

  <div class="flex items-center gap-2 shrink-0">
    {#if extraActions}
      {@render extraActions()}
    {/if}

    {#if onAction}
      <Button
        variant={rebuildBytes > 0 ? "secondary" : "primary"}
        size="sm"
        disabled={isActionDisabled || isActionLoading || selectedCount === 0 || manualCount > 0}
        title={manualCount > 0 ? "These items need a separate action" : undefined}
        onclick={onAction}
        class="gap-1.5"
      >
        <ListChecks size={13} />
        <span>{isActionLoading ? "Working…" : actionLabel}</span>
      </Button>
    {/if}
  </div>
</div>
