<script lang="ts">
  import { onMount } from 'svelte';
  import type { PlanPreview } from '../models/types';
  import { formatBytes } from '../utils/format';
  import Button from './Button.svelte';
  import { isFocusable, restoreFocus } from '../utils/focus';
  import { platformContextStore } from '../stores/platformContext.svelte';

  let { plan, disabled = false, onCancel, onConfirm, returnFocusTarget }: {
    plan: PlanPreview;
    disabled?: boolean;
    onCancel: () => void;
    onConfirm: () => void;
    returnFocusTarget?: HTMLElement | null;
  } = $props();
  const id = $props.id();
  let dialog: HTMLDialogElement;
  let hasRebuild = $derived(plan.targets.some(target => target.risk === 'rebuild'));
  let hasUnknownEstimate = $derived(plan.targets.some(target => target.expected_bytes === 0));
  let actionSummary = $derived(plan.mode === 'trash'
    ? `Move to ${platformContextStore.trashLabel}`
    : plan.mode === 'mixed'
      ? `Some items move to ${platformContextStore.trashLabel}; others are deleted`
      : 'Delete permanently');
  let isConfirmed = false;

  onMount(() => {
    const previousFocus = document.activeElement as HTMLElement | null;
    if (typeof dialog?.showModal === 'function') {
      dialog.showModal();
    }
    return () => {
      if (dialog?.open) {
        dialog.close();
      }
      if (!isConfirmed) {
        const preferred = isFocusable(returnFocusTarget) ? returnFocusTarget : previousFocus;
        restoreFocus(preferred);
      }
    };
  });

  function handleConfirm() {
    isConfirmed = true;
    onConfirm();
  }

  function handleCancel() {
    onCancel();
  }
</script>

<dialog
  bind:this={dialog}
  id={id + '-dialog'}
  aria-modal="true"
  aria-labelledby={id + '-title'}
  aria-describedby={id + '-description'}
  oncancel={(event) => { event.preventDefault(); handleCancel(); }}
  class="m-auto w-[calc(100%-2rem)] max-w-lg max-h-[calc(100%-2rem)] overflow-y-auto scroll-stable rounded-2xl border border-border bg-card p-5 text-foreground shadow-xl backdrop:bg-foreground/30 focus:outline-none"
>
  <h2 id={id + '-title'} class="text-title font-semibold tracking-tight">Clean {plan.targets.length} {plan.targets.length === 1 ? 'item' : 'items'}?</h2>
  <p id={id + '-description'} class="mt-2 text-body text-muted-foreground">
    {#if hasUnknownEstimate && plan.expected_reclaim_bytes === 0}
      {actionSummary} · reclaimed amount depends on what the owner tool can prune
    {:else}
      {actionSummary} · {formatBytes(plan.expected_reclaim_bytes)} estimated
      {#if hasUnknownEstimate}<br />Owner-managed prune amounts may vary.{/if}
    {/if}
  </p>
  {#if hasRebuild}
    <p class="mt-2 text-meta text-warning">Some items may download or build again later.</p>
  {/if}
  <ul class="my-4 divide-y divide-border">
    {#each plan.targets as target (target.item_id)}
      <li class="flex flex-wrap items-center justify-between gap-2 py-2 text-body">
        <div class="min-w-0 flex-1 break-words">
          <span>{target.name}</span>
          <code class="mt-0.5 block break-all text-meta text-muted-foreground">{target.path}</code>
        </div>
        <span class="shrink-0 whitespace-nowrap font-mono tabular-nums">
          {target.expected_bytes === 0 ? 'Varies' : formatBytes(target.expected_bytes)}
        </span>
      </li>
    {/each}
  </ul>
  {#if disabled}
    <p role="status" class="mb-3 text-meta text-warning">This selection expired. Scan again.</p>
  {/if}
  <div class="flex flex-wrap justify-end gap-2">
    <Button variant="secondary" onclick={handleCancel}>Cancel</Button>
    <Button variant="destructive" disabled={disabled || plan.targets.length === 0} onclick={handleConfirm}>Clean items</Button>
  </div>
</dialog>
