<script lang="ts">
  import type { CleanResult } from '../models/types';
  import { formatBytes } from '../utils/format';
  import Button from './Button.svelte';
  import Card from './Card.svelte';
  import Badge from './Badge.svelte';
  import { CheckCircle2, AlertTriangle, X } from 'lucide-svelte';

  interface Props {
    result: CleanResult;
    onClose: () => void;
  }

  let { result, onClose }: Props = $props();

  let failedItems = $derived(result.items.filter((i) => !i.success));
  let successItems = $derived(result.items.filter((i) => i.success));
</script>

<div
  class="fixed inset-0 z-50 bg-background/80 backdrop-blur-sm flex items-center justify-center p-4"
>
  <Card class="w-full max-w-md bg-card shadow-2xl border-border animate-in fade-in zoom-in-95">
    <div class="flex items-center justify-between pb-3 border-b border-border/80">
      <div class="flex items-center gap-2">
        <div class="h-8 w-8 rounded-full bg-emerald-500/20 text-emerald-400 flex items-center justify-center">
          <CheckCircle2 size={18} />
        </div>
        <div>
          <h3 class="text-sm font-semibold text-foreground">Clean Complete</h3>
          <p class="text-xs text-muted-foreground">Storage has been safely reclaimed</p>
        </div>
      </div>
      <Button variant="ghost" size="icon" onclick={onClose} ariaLabel="Close cleanup result" title="Close">
        <X size={16} />
      </Button>
    </div>

    <div class="py-4 space-y-4">
      <div class="p-3 bg-secondary/50 rounded-lg text-center">
        <div class="text-2xl font-bold font-mono text-foreground">
          {formatBytes(result.total_reclaimed_bytes)}
        </div>
        <div class="text-xs text-muted-foreground mt-0.5">
          Disk Space Reclaimed
          {#if result.actual_disk_free_delta}
            <span class="text-emerald-500 ml-1">
              (Free space delta: +{formatBytes(Math.max(0, result.actual_disk_free_delta))})
            </span>
          {/if}
        </div>
      </div>

      {#if failedItems.length > 0}
        <div class="space-y-2">
          <div class="flex items-center gap-1.5 text-xs font-medium text-amber-500">
            <AlertTriangle size={14} />
            <span>{failedItems.length} items skipped or failed</span>
          </div>
          <div class="max-h-32 overflow-y-auto space-y-1.5 pr-1">
            {#each failedItems as item}
              <div class="p-2 rounded bg-destructive/10 border border-destructive/20 text-xs">
                <div class="font-medium text-foreground">{item.name}</div>
                <div class="text-[11px] text-muted-foreground mt-0.5">
                  {item.error_message || 'Could not clean item'}
                </div>
              </div>
            {/each}
          </div>
        </div>
      {/if}

      <div class="space-y-1.5 max-h-48 overflow-y-auto pr-1">
        <span class="text-xs font-medium text-muted-foreground">Cleaned Items ({successItems.length})</span>
        {#each successItems as item}
          <div class="flex items-center justify-between py-1 text-xs border-b border-border/40 last:border-0">
            <span class="truncate text-foreground max-w-[240px]">{item.name}</span>
            <span class="font-mono text-muted-foreground">
              {formatBytes(item.bytes_reclaimed)}
            </span>
          </div>
        {/each}
      </div>
    </div>

    <div class="pt-2 flex justify-end">
      <Button variant="primary" size="md" onclick={onClose} class="w-full">
        Done
      </Button>
    </div>
  </Card>
</div>
