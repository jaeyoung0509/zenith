<script lang="ts">
  import type { ScanItem } from '../models/types';
  import { formatBytes, formatTimeAgo } from '../utils/format';
  import { scanStore } from '../stores/scan.svelte';
  import { tauriShowInFileManager } from '../utils/tauri';
  import RiskBadge from './RiskBadge.svelte';
  import Button from './Button.svelte';
  import Checkbox from './Checkbox.svelte';
  import { FolderOpen, ArrowUpRight } from 'lucide-svelte';

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
  let isManual = $derived(item.risk === 'manual' || cacheMetadata.management_mode === 'advisory');
  let isSelected = $derived(!!scanStore.selectedMap[item.id] && !isManual);

  function handleToggle() {
    if (isManual) return;
    scanStore.toggleItem(item.id);
  }

  function handleReveal(e: MouseEvent) {
    e.stopPropagation();
    tauriShowInFileManager(item.path);
  }
</script>

<div
  class="flex items-center justify-between p-3 rounded-lg border border-border/60 hover:border-border hover:bg-secondary/40 transition-colors group {isSelected
    ? 'bg-secondary/30'
    : ''}"
>
  <div class="flex items-start space-x-3 flex-1 min-w-0 pr-3">
    {#if isManual}
      <button
        type="button"
        onclick={handleReveal}
        title="Manual item: requires dedicated adapter or review"
        class="mt-0.5 px-1.5 py-0.5 rounded text-caption font-medium border border-destructive/30 text-destructive bg-destructive/10 flex items-center gap-0.5 shrink-0 hover:bg-destructive/20 transition-colors cursor-pointer"
      >
        <span>Manual</span>
        <ArrowUpRight size={10} />
      </button>
    {:else}
      <Checkbox
        checked={isSelected}
        onchange={handleToggle}
        ariaLabel={`Select ${item.name}`}
        class="mt-0.5 shrink-0"
      />
    {/if}

    <div class="flex-1 min-w-0">
      <div class="flex items-center gap-2">
        <span class="text-xs font-medium text-foreground truncate">
          {item.name}
        </span>
        <RiskBadge risk={item.risk} />
      </div>

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
    <span class="min-w-[5rem] whitespace-nowrap text-right text-xs font-mono font-semibold text-foreground">
      {cacheMetadata.size_semantics === 'conservative_lower_bound' ? '≥ ' : cacheMetadata.size_semantics === 'informational' ? '~ ' : ''}{formatBytes(item.size.allocated ?? item.size.logical)}
    </span>

    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7 opacity-0 group-hover:opacity-100 transition-opacity"
      onclick={handleReveal}
      ariaLabel={`Show ${item.name} in file manager`}
      title={`Show ${item.name} in file manager`}
    >
      <FolderOpen size={13} class="text-muted-foreground" />
    </Button>
  </div>
</div>
