<script lang="ts">
  import type { Snippet } from "svelte";
  import Button from "./Button.svelte";
  import ByteValue from "./ByteValue.svelte";
  import { CheckSquare, Square, Trash2 } from "lucide-svelte";

  interface Props {
    selectedCount: number;
    totalCount?: number;
    selectedBytes?: number;
    safeBytes?: number;
    rebuildBytes?: number;
    manualBytes?: number;
    onSelectAll?: () => void;
    onSelectSafe?: () => void;
    onDeselectAll?: () => void;
    actionLabel?: string;
    onAction?: () => void;
    isActionDisabled?: boolean;
    isActionLoading?: boolean;
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
    onSelectAll,
    onSelectSafe,
    onDeselectAll,
    actionLabel = "Clean Selected",
    onAction,
    isActionDisabled = false,
    isActionLoading = false,
    class: className = "",
    extraActions,
  }: Props = $props();
</script>

<div
  class="flex flex-col sm:flex-row sm:items-center justify-between gap-3 p-3 rounded-xl border border-border/80 bg-card/80 shadow-xs {className}"
>
  <div class="flex flex-wrap items-center gap-2">
    {#if onSelectSafe}
      <Button
        variant="ghost"
        size="xs"
        onclick={onSelectSafe}
        class="text-xs text-muted-foreground hover:text-foreground"
      >
        <CheckSquare size={13} class="mr-1 text-success" />
        <span>Select Safe</span>
      </Button>
    {/if}

    {#if onSelectAll}
      <Button
        variant="ghost"
        size="xs"
        onclick={onSelectAll}
        class="text-xs text-muted-foreground hover:text-foreground"
      >
        <span>Select All</span>
      </Button>
    {/if}

    {#if onDeselectAll}
      <Button
        variant="ghost"
        size="xs"
        disabled={selectedCount === 0}
        onclick={onDeselectAll}
        class="text-xs text-muted-foreground hover:text-foreground"
      >
        <Square size={13} class="mr-1" />
        <span>Deselect</span>
      </Button>
    {/if}

    <div class="h-4 w-px bg-border/60 mx-1 hidden sm:block"></div>

    <span class="text-xs text-muted-foreground">
      <span class="font-medium text-foreground">{selectedCount}</span>
      {#if totalCount !== undefined}
        <span class="text-muted-foreground/70"> of {totalCount}</span>
      {/if}
      selected
      {#if selectedBytes > 0}
        · <span class="font-mono font-semibold text-foreground"><ByteValue bytes={selectedBytes} /></span>
      {/if}
    </span>

    {#if safeBytes > 0 || rebuildBytes > 0}
      <span class="inline-flex items-center gap-1.5 text-caption font-mono">
        {#if safeBytes > 0}
          <span class="text-success">✓ <ByteValue bytes={safeBytes} /> safe</span>
        {/if}
        {#if rebuildBytes > 0}
          <span class="text-warning">↻ <ByteValue bytes={rebuildBytes} /> rebuild</span>
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
        disabled={isActionDisabled || selectedCount === 0}
        onclick={onAction}
        class="gap-1.5"
      >
        <Trash2 size={13} />
        <span>{actionLabel}</span>
      </Button>
    {/if}
  </div>
</div>
