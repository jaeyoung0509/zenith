<script lang="ts">
  import type { ScanItem } from '../models/types';
  import { formatBytes, formatTimeAgo } from '../utils/format';
  import { isActionable, isAdvisory } from '../utils/cleanup';
  import { scanStore } from '../stores/scan.svelte';
  import { platformContextStore } from '../stores/platformContext.svelte';
  import { canReveal, revealUnavailableReason, runReveal } from '../utils/reveal';
  import { tauriShowInFileManager } from '../utils/tauri';
  import Button from './Button.svelte';
  import Checkbox from './Checkbox.svelte';
  import { FolderOpen, ArrowUpRight } from '@lucide/svelte';

  interface Props {
    item: ScanItem;
  }

  let { item }: Props = $props();

  let cacheMetadata = $derived(item.cache_metadata ?? {
    provider: 'Zenith',
    management_mode: 'zenith' as const,
    artifact_kind: 'temporary' as const,
    consequence: '',
    size_semantics: 'physical_reclaimable' as const,
    last_used_confidence: 'unknown' as const,
  });
  let cleanable = $derived(isActionable(item));
  let isAdvisoryItem = $derived(isAdvisory(item));
  let isSelected = $derived(!!scanStore.selectedMap[item.id] && cleanable);
  let blockedReason = $derived(item.disposition?.reason ?? item.incomplete_reason ?? 'Cleanup blocked');
  // A refusal names this row and stays non-blocking: the row keeps its own
  // eligibility, and the reason states what the last cleanup did not cover.
  let refusalReason = $derived(scanStore.refusedItems[item.id] ?? null);

  // An item that cannot be cleaned must not present a reclaimable amount: its
  // size is informational, exactly like an `informational` size semantics.
  let sizePrefix = $derived.by(() => {
    if (!cleanable || cacheMetadata.size_semantics === 'informational') return '~ ';
    if (item.quality === 'partial' || cacheMetadata.size_semantics === 'conservative_lower_bound') {
      return '≥ ';
    }
    return '';
  });
  let revealError = $state<string | null>(null);

  function handleToggle() {
    if (!cleanable) return;
    scanStore.toggleItem(item.id);
  }

  function handleReveal(e: MouseEvent) {
    e.stopPropagation();
    revealError = null;
    void runReveal(() => tauriShowInFileManager(item.path), (message) => (revealError = message));
  }
</script>

<div
  class="group flex min-h-[56px] items-center justify-between gap-3 rounded-xl px-3 py-2 transition-ui {isSelected
    ? 'bg-accent'
    : 'hover:bg-accent/40'}"
>
  <div class="flex items-start space-x-3 flex-1 min-w-0 pr-3">
    {#if isAdvisoryItem}
      <button
        type="button"
        onclick={handleReveal}
        disabled={!canReveal()}
        title={canReveal() ? platformContextStore.revealLabel : revealUnavailableReason()}
        class="mt-0.5 inline-flex h-7 items-center gap-1 shrink-0 rounded-md border border-border-strong bg-transparent px-2 text-caption font-medium text-muted-foreground transition-ui hover:bg-accent hover:text-foreground cursor-pointer disabled:opacity-45 disabled:cursor-not-allowed"
      >
        <span>Show location</span>
        <ArrowUpRight size={10} />
      </button>
    {:else}
      <Checkbox
        checked={isSelected}
        disabled={!scanStore.canClean || !cleanable}
        onchange={handleToggle}
        ariaLabel={!cleanable ? `${item.name}: ${blockedReason}` : `Select ${item.name}`}
        title={!cleanable ? blockedReason : undefined}
        class="mt-0.5 shrink-0"
      />
    {/if}

    <div class="flex-1 min-w-0">
      <div class="flex items-center gap-2">
        <span class="text-body font-medium text-foreground break-words">
          {item.name}
        </span>
      </div>

      {#if refusalReason}
        <p class="text-meta text-warning mt-0.5 line-clamp-1" title={refusalReason}>
          {refusalReason}
        </p>
      {:else if item.disposition?.reason}
        <p class="text-meta text-warning mt-0.5 line-clamp-1" title={item.disposition.reason}>
          {item.disposition.reason}
        </p>
      {:else if item.incomplete_reason}
        <p class="text-meta text-warning mt-0.5 line-clamp-1" title={item.incomplete_reason}>
          {item.incomplete_reason}
        </p>
      {:else if item.risk === 'rebuild' && cleanable}
        <p class="text-meta text-muted-foreground mt-0.5">May download or build again later</p>
      {/if}

      <p class="mt-0.5 truncate text-meta text-muted-foreground">
        {#if item.description}{item.description} · {/if}<span class="font-mono text-caption">{item.path}</span>{#if item.file_count > 0} · {item.file_count} files{/if}{#if item.last_modified} · modified {formatTimeAgo(item.last_modified)}{/if}
      </p>
    </div>
  </div>

  <div class="flex shrink-0 items-center gap-2">
    {#if revealError}
      <span role="alert" class="text-meta text-destructive">{revealError}</span>
    {/if}
    <span class="w-[12ch] shrink-0 whitespace-nowrap text-right text-body font-mono tabular-nums font-semibold text-foreground">
      {sizePrefix}{formatBytes(item.size.allocated ?? item.size.logical)}
    </span>

    <Button
      variant="ghost"
      size="icon"
      class="opacity-0 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity"
      disabled={!canReveal()}
      onclick={handleReveal}
      ariaLabel={`Show ${item.name} in file manager`}
      title={canReveal() ? platformContextStore.revealLabel : revealUnavailableReason()}
    >
      <FolderOpen size={14} class="text-muted-foreground" />
    </Button>
  </div>
</div>
