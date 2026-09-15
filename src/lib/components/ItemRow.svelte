<script lang="ts">
  import type { ScanItem } from '../models/types';
  import { formatBytes, formatTimeAgo } from '../utils/format';
  import { isAdvisory, isBlocked, isCleanable, isPolicyGated, isRecent } from '../utils/cleanup';
  import { scanStore } from '../stores/scan.svelte';
  import { platformContextStore } from '../stores/platformContext.svelte';
  import { canReveal, revealUnavailableReason, runReveal } from '../utils/reveal';
  import { tauriShowInFileManager } from '../utils/tauri';
  import RiskBadge from './RiskBadge.svelte';
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
  let cleanable = $derived(isCleanable(item));
  let isBlockedItem = $derived(isBlocked(item));
  let isAdvisoryItem = $derived(isAdvisory(item));
  let isSelected = $derived(!!scanStore.selectedMap[item.id] && cleanable);
  let blockedReason = $derived(item.disposition?.reason ?? item.incomplete_reason ?? 'Cleanup blocked');

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
  class="flex items-center justify-between p-3 rounded-lg border border-border/60 hover:border-border hover:bg-secondary/40 transition-colors group {isSelected
    ? 'bg-secondary/30'
    : ''}"
>
  <div class="flex items-start space-x-3 flex-1 min-w-0 pr-3">
    {#if isAdvisoryItem}
      <button
        type="button"
        onclick={handleReveal}
        disabled={!canReveal()}
        title={canReveal() ? platformContextStore.revealLabel : revealUnavailableReason()}
        class="mt-0.5 px-1.5 py-0.5 rounded text-caption font-medium border border-destructive/30 text-destructive bg-destructive/10 flex items-center gap-0.5 shrink-0 hover:bg-destructive/20 transition-colors cursor-pointer disabled:opacity-45 disabled:cursor-not-allowed"
      >
        <span>Manual</span>
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
        <span class="text-xs font-medium text-foreground truncate">
          {item.name}
        </span>
        <RiskBadge risk={item.risk} />
        {#if item.disposition?.eligibility === 'blocked'}
          <span class="px-1.5 py-0.5 rounded text-micro font-medium border border-destructive/40 text-destructive bg-destructive/10" title={blockedReason}>
            Blocked
          </span>
        {:else if item.disposition?.eligibility === 'advisory'}
          <span class="px-1.5 py-0.5 rounded text-micro font-medium border border-warning/40 text-warning bg-warning/10" title={blockedReason}>
            Advisory
          </span>
        {:else if isRecent(item)}
          <span class="px-1.5 py-0.5 rounded text-micro font-medium border border-warning/40 text-warning bg-warning/10" title={item.disposition?.reason ?? blockedReason}>
            Recently used
          </span>
        {:else if isPolicyGated(item)}
          <span class="px-1.5 py-0.5 rounded text-micro font-medium border border-warning/40 text-warning bg-warning/10" title={item.disposition?.reason ?? blockedReason}>
            Outside scope
          </span>
        {:else if item.quality === 'partial'}
          <span class="px-1.5 py-0.5 rounded text-micro font-medium border border-warning/40 text-warning bg-warning/10" title={item.incomplete_reason ?? 'Partial scan'}>
            Partial
          </span>
        {:else if isBlockedItem}
          <span class="px-1.5 py-0.5 rounded text-micro font-medium border border-destructive/40 text-destructive bg-destructive/10" title={blockedReason}>
            Inaccessible
          </span>
        {/if}
      </div>

      {#if item.disposition?.reason}
        <p class="text-caption text-warning mt-0.5 line-clamp-1" title={item.disposition.reason}>
          {item.disposition.reason}
        </p>
      {:else if item.incomplete_reason}
        <p class="text-caption text-warning mt-0.5 line-clamp-1" title={item.incomplete_reason}>
          {item.incomplete_reason}
        </p>
      {/if}

      <p class="text-meta text-muted-foreground mt-0.5 line-clamp-1">
        {item.description || item.path}
      </p>

      {#if cacheMetadata.provider || cacheMetadata.consequence}
        <div class="flex flex-wrap items-center gap-1.5 mt-1 text-caption text-muted-foreground">
          <span class="rounded border border-border/70 px-1.5 py-0.5">
            {cacheMetadata.provider || 'Zenith'} · {cacheMetadata.management_mode.replace('_', ' ')}
          </span>
          <span class="rounded border border-border/70 px-1.5 py-0.5">
            {cacheMetadata.artifact_kind.replaceAll('_', ' ')}
          </span>
          {#if cacheMetadata.consequence}
            <span class="line-clamp-1">{cacheMetadata.consequence}</span>
          {/if}
          {#if cacheMetadata.last_used_confidence !== 'unknown'}
            <span>usage time: {cacheMetadata.last_used_confidence}</span>
          {/if}
        </div>
      {/if}

      <div class="flex items-center gap-2 mt-1 text-caption text-muted-foreground font-mono">
        <span class="truncate max-w-[280px]">{item.path}</span>
        {#if item.file_count > 0}
          <span>• {item.file_count} files</span>
        {/if}
        {#if item.last_modified}
          <span>• modified {formatTimeAgo(item.last_modified)}</span>
        {/if}
      </div>
    </div>
  </div>

  <div class="flex items-center gap-2 shrink-0">
    {#if revealError}
      <span role="alert" class="text-caption text-destructive">{revealError}</span>
    {/if}
    <span class="w-[12ch] whitespace-nowrap text-right text-xs font-mono tabular-nums font-semibold text-foreground">
      {sizePrefix}{formatBytes(item.size.allocated ?? item.size.logical)}
    </span>

    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7 opacity-0 group-hover:opacity-100 transition-opacity"
      disabled={!canReveal()}
      onclick={handleReveal}
      ariaLabel={`Show ${item.name} in file manager`}
      title={canReveal() ? platformContextStore.revealLabel : revealUnavailableReason()}
    >
      <FolderOpen size={13} class="text-muted-foreground" />
    </Button>
  </div>
</div>
