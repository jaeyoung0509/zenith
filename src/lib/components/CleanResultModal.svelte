<script lang="ts">
  import { fade, scale } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { prefersReducedMotion } from 'svelte/motion';
  import type { CleanResult } from '../models/types';
  import { formatBytes } from '../utils/format';
  import Button from './Button.svelte';
  import Card from './Card.svelte';
  import { CheckCircle2, AlertTriangle, X, AlertCircle } from 'lucide-svelte';

  interface Props {
    result: CleanResult;
    onClose: () => void;
  }

  let { result, onClose }: Props = $props();

  let failedItems = $derived(
    result.items.filter((i) => !i.success || i.status === 'failed')
  );
  let partialItems = $derived(
    result.items.filter((i) => i.status === 'partial' || (i.success && !!i.error_message))
  );
  let fullSuccessItems = $derived(
    result.items.filter((i) => i.status === 'success' || (i.success && !i.error_message && i.status !== 'partial'))
  );
</script>

<div
  transition:fade={{ duration: prefersReducedMotion.current ? 0 : 140 }}
  class="fixed inset-0 z-50 bg-background/80 backdrop-blur-sm flex items-center justify-center p-4"
>
  <div
    transition:scale={{
      duration: prefersReducedMotion.current ? 0 : 180,
      start: prefersReducedMotion.current ? 1 : 0.96,
      easing: cubicOut,
    }}
    class="w-full max-w-md"
  >
    <Card class="w-full bg-card shadow-2xl border-border">
    <div class="flex items-center justify-between pb-3 border-b border-border/80">
      <div class="flex items-center gap-2">
        <div class="h-8 w-8 rounded-full bg-success/20 text-success flex items-center justify-center">
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
            <span class="text-success ml-1">
              (Free space delta: +{formatBytes(Math.max(0, result.actual_disk_free_delta))})
            </span>
          {/if}
        </div>
      </div>

      <!-- Failed Items -->
      {#if failedItems.length > 0}
        <div class="space-y-1.5">
          <div class="flex items-center gap-1.5 text-xs font-medium text-destructive">
            <AlertCircle size={14} />
            <span>{failedItems.length} item(s) failed</span>
          </div>
          <div class="max-h-28 overflow-y-auto space-y-1.5 pr-1">
            {#each failedItems as item}
              <div class="p-2 rounded bg-destructive/10 border border-destructive/20 text-xs">
                <div class="font-medium text-foreground">{item.name}</div>
                <div class="text-meta text-muted-foreground mt-0.5">
                  {item.error_message || 'Could not clean item'}
                </div>
              </div>
            {/each}
          </div>
        </div>
      {/if}

      <!-- Partial Items -->
      {#if partialItems.length > 0}
        <div class="space-y-1.5">
          <div class="flex items-center gap-1.5 text-xs font-medium text-warning">
            <AlertTriangle size={14} />
            <span>{partialItems.length} item(s) partially cleaned</span>
          </div>
          <div class="max-h-28 overflow-y-auto space-y-1.5 pr-1">
            {#each partialItems as item}
              <div class="p-2 rounded bg-warning/10 border border-warning/20 text-xs">
                <div class="flex items-center justify-between">
                  <span class="font-medium text-foreground">{item.name}</span>
                  <span class="font-mono text-warning text-meta">+{formatBytes(item.bytes_reclaimed)}</span>
                </div>
                <div class="text-meta text-warning/80 mt-0.5">
                  {item.error_message || 'Some files were locked or in use'}
                </div>
              </div>
            {/each}
          </div>
        </div>
      {/if}

      <!-- Fully Cleaned Items -->
      {#if fullSuccessItems.length > 0}
        <div class="space-y-1.5 max-h-40 overflow-y-auto pr-1">
          <span class="text-xs font-medium text-muted-foreground">Cleaned Items ({fullSuccessItems.length})</span>
          {#each fullSuccessItems as item}
            <div class="flex items-center justify-between py-1 text-xs border-b border-border/40 last:border-0">
              <span class="truncate text-foreground max-w-[240px]">{item.name}</span>
              <span class="font-mono text-muted-foreground">
                {formatBytes(item.bytes_reclaimed)}
              </span>
            </div>
          {/each}
        </div>
      {/if}
    </div>

    <div class="pt-2 flex justify-end">
      <Button variant="primary" size="md" onclick={onClose} class="w-full">
        Done
      </Button>
    </div>
  </Card>
  </div>
</div>
