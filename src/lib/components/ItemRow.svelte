<script lang="ts">
  import type { ScanItem } from '../models/types';
  import { formatBytes, formatTimeAgo } from '../utils/format';
  import { scanStore } from '../stores/scan.svelte';
  import { tauriRevealInFinder } from '../utils/tauri';
  import RiskBadge from './RiskBadge.svelte';
  import Button from './Button.svelte';
  import { FolderOpen } from 'lucide-svelte';

  interface Props {
    item: ScanItem;
  }

  let { item }: Props = $props();

  let isManual = $derived(item.risk === 'manual');
  let isSelected = $derived(!!scanStore.selectedMap[item.id] && !isManual);

  function handleToggle() {
    if (isManual) return;
    scanStore.toggleItem(item.id);
  }

  function handleReveal(e: MouseEvent) {
    e.stopPropagation();
    tauriRevealInFinder(item.path);
  }
</script>

<div
  class="flex items-center justify-between p-3 rounded-lg border border-border/60 hover:border-border hover:bg-secondary/40 transition-colors group {isSelected
    ? 'bg-secondary/30'
    : ''}"
>
  <div class="flex items-start space-x-3 flex-1 min-w-0 pr-3">
    {#if isManual}
      <div
        class="mt-1 h-4 w-4 rounded border border-border/40 bg-secondary/30 flex items-center justify-center shrink-0 opacity-40 cursor-not-allowed"
        title="Manual item: requires dedicated adapter (manage in Local Models or Containers view)"
      ></div>
    {:else}
      <input
        type="checkbox"
        checked={isSelected}
        onchange={handleToggle}
        class="mt-1 h-4 w-4 rounded border-border text-primary focus:ring-ring cursor-pointer accent-primary shrink-0"
      />
    {/if}

    <div class="flex-1 min-w-0">
      <div class="flex items-center gap-2">
        <span class="text-xs font-medium text-foreground truncate">
          {item.name}
        </span>
        <RiskBadge risk={item.risk} />
      </div>

      <p class="text-[11px] text-muted-foreground mt-0.5 line-clamp-1">
        {item.description || item.path}
      </p>

      <div class="flex items-center gap-2 mt-1 text-[10px] text-muted-foreground font-mono">
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
    <span class="text-xs font-mono font-semibold text-foreground">
      {formatBytes(item.size.allocated ?? item.size.logical)}
    </span>

    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7 opacity-0 group-hover:opacity-100 transition-opacity"
      onclick={handleReveal}
    >
      <FolderOpen size={13} class="text-muted-foreground" />
    </Button>
  </div>
</div>
