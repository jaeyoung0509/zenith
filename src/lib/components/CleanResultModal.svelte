<script lang="ts">
  import { onMount } from 'svelte';
  import type { CleanResult } from '../models/types';
  import { formatBytes } from '../utils/format';
  import { cleanOutcome } from '../utils/cleanResult';
  import Button from './Button.svelte';
  import { CheckCircle2, AlertTriangle, X, AlertCircle } from 'lucide-svelte';
  import { restoreFocus } from '../utils/focus';

  interface Props {
    result: CleanResult;
    onClose: () => void;
    returnFocusTarget?: HTMLElement | null;
    returnFocusTargetId?: string;
  }

  let { result, onClose, returnFocusTarget, returnFocusTargetId }: Props = $props();
  const id = $props.id();
  let dialog: HTMLDialogElement;

  let outcome = $derived(cleanOutcome(result));
  let failedItems = $derived(
    result.items.filter((i) => !i.success || i.status === 'failed')
  );
  let partialItems = $derived(
    result.items.filter(
      (i) => i.success && (i.status === 'partial' || !!i.error_message)
    )
  );
  let fullSuccessItems = $derived(
    result.items.filter((i) => i.success && i.status === 'success' && !i.error_message)
  );

  onMount(() => {
    const previousFocus = document.activeElement as HTMLElement | null;
    if (typeof dialog?.showModal === 'function') {
      dialog.showModal();
    }
    const doneBtn = dialog?.querySelector<HTMLButtonElement>(`#${id}-done-button`);
    doneBtn?.focus();

    return () => {
      if (dialog?.open) {
        dialog.close();
      }
      const explicitTarget = returnFocusTargetId
        ? document.getElementById(returnFocusTargetId)
        : returnFocusTarget;
      restoreFocus(explicitTarget ?? previousFocus);
    };
  });

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key === 'Tab') {
      const focusable = dialog?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
      );
      if (!focusable || focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }
  }

  function handleBackdropClick(event: MouseEvent) {
    if (event.target === dialog) {
      const rect = dialog.getBoundingClientRect();
      const isInDialog =
        rect.top <= event.clientY &&
        event.clientY <= rect.top + rect.height &&
        rect.left <= event.clientX &&
        event.clientX <= rect.left + rect.width;
      if (!isInDialog) {
        onClose();
      }
    }
  }
</script>

<dialog
  bind:this={dialog}
  id={id + '-dialog'}
  aria-modal="true"
  aria-labelledby={id + '-title'}
  aria-describedby={id + '-description'}
  oncancel={(event) => { event.preventDefault(); onClose(); }}
  onclick={handleBackdropClick}
  onkeydown={handleKeydown}
  class="m-auto w-[calc(100%-2rem)] max-w-md max-h-[calc(100%-2rem)] overflow-y-auto scroll-stable rounded-xl border border-border bg-card p-5 text-foreground shadow-2xl backdrop:bg-background/80 backdrop:backdrop-blur-sm focus:outline-none"
>
  <div class="flex items-center justify-between pb-3 border-b border-border/80">
    <div class="flex items-center gap-2">
      <div
        class={`h-8 w-8 rounded-full flex items-center justify-center ${
          outcome === 'success'
            ? 'bg-success/20 text-success'
            : outcome === 'partial'
              ? 'bg-warning/20 text-warning'
              : 'bg-destructive/20 text-destructive'
        }`}
      >
        {#if outcome === 'success'}
          <CheckCircle2 size={18} />
        {:else if outcome === 'partial'}
          <AlertTriangle size={18} />
        {:else}
          <AlertCircle size={18} />
        {/if}
      </div>
      <div>
        <h3 id={id + '-title'} class="text-sm font-semibold text-foreground">
          {outcome === 'success'
            ? 'Clean Complete'
            : outcome === 'partial'
              ? 'Clean Partially Complete'
              : 'Clean Failed'}
        </h3>
        <p id={id + '-description'} class="text-xs text-muted-foreground">
          {outcome === 'success'
            ? 'Storage has been safely reclaimed'
            : outcome === 'partial'
              ? 'Some storage was reclaimed; review the remaining items'
              : 'No storage was reclaimed; review the errors below'}
        </p>
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
        {#if outcome !== 'failed' && result.actual_disk_free_delta != null && result.actual_disk_free_delta > 0}
          <span class="text-success ml-1">
            (Free space delta: +{formatBytes(result.actual_disk_free_delta)})
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
        <div class="max-h-28 overflow-y-auto scroll-stable space-y-1.5">
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
        <div class="max-h-28 overflow-y-auto scroll-stable space-y-1.5">
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
      <div class="space-y-1.5 max-h-40 overflow-y-auto scroll-stable">
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
    <Button id={id + '-done-button'} variant="primary" size="md" onclick={onClose} class="w-full">
      Done
    </Button>
  </div>
</dialog>
