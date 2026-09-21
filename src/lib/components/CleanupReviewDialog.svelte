<script lang="ts">
  import { onMount } from 'svelte';
  import type { PlanPreview, ScanItem } from '../models/types';
  import { formatBytes } from '../utils/format';
  import Button from './Button.svelte';
  import RiskBadge from './RiskBadge.svelte';
  import { isFocusable, restoreFocus } from '../utils/focus';
  import { platformContextStore } from '../stores/platformContext.svelte';

  let { plan, items = [], disabled = false, onCancel, onConfirm, returnFocusTarget }: {
    plan: PlanPreview;
    items?: ScanItem[];
    disabled?: boolean;
    onCancel: () => void;
    onConfirm: () => void;
    returnFocusTarget?: HTMLElement | null;
  } = $props();
  const id = $props.id();
  let dialog: HTMLDialogElement;
  let hasRebuild = $derived(plan.targets.some(target => target.risk === 'rebuild'));
  let consequenceById = $derived(
    new Map(items.map(item => [item.id, item.cache_metadata?.consequence ?? null]))
  );
  let actionSummary = $derived(
    plan.mode === 'trash'
      ? `These items move to ${platformContextStore.trashLabel} and remain recoverable until it is emptied.`
      : plan.mode === 'mixed'
        ? `Some targets are permanently cleaned; recoverable filesystem targets move to ${platformContextStore.trashLabel}.`
        : `These actions do not use ${platformContextStore.trashLabel} and cannot be restored through Zenith.`
  );
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
  class="m-auto w-[calc(100%-2rem)] max-w-lg max-h-[calc(100%-2rem)] overflow-y-auto scroll-stable rounded-xl border border-border bg-card p-5 text-foreground shadow-xl backdrop:bg-black/50 focus:outline-none"
>
  <h2 id={id + '-title'} class="text-base font-semibold">Review cleanup</h2>
  <p id={id + '-description'} class="mt-2 text-sm text-muted-foreground">
    Review these {plan.targets.length} backend-verified targets before continuing. {actionSummary}
  </p>
  {#if hasRebuild}
    <p class="mt-2 text-sm text-warning">Rebuild items may require downloads or recompilation the next time you use the tool.</p>
  {/if}
  <ul class="my-4 divide-y divide-border">
    {#each plan.targets as target (target.item_id)}
      <li class="flex flex-wrap items-center justify-between gap-2 py-2 text-sm">
        <div class="min-w-0 flex-1 break-words">
          <span>{target.name}</span>
          <code class="mt-0.5 block break-all text-meta text-muted-foreground">{target.path}</code>
          {#if consequenceById.get(target.item_id)}
            <span class="mt-0.5 block text-meta text-muted-foreground">{consequenceById.get(target.item_id)}</span>
          {/if}
        </div>
        <span class="flex items-center gap-2"><span class="text-meta text-muted-foreground">{target.mode === 'trash' ? platformContextStore.trashLabel : 'Permanent'}</span><RiskBadge risk={target.risk} /><span class="font-mono whitespace-nowrap">{formatBytes(target.expected_bytes)}</span></span>
      </li>
    {/each}
  </ul>
  {#if disabled}
    <p role="status" class="mb-3 text-sm text-warning">The scan changed or expired. Cancel and review a fresh selection.</p>
  {/if}
  <div class="flex flex-wrap justify-end gap-2">
    <Button variant="secondary" onclick={handleCancel}>Cancel</Button>
    <Button variant="destructive" disabled={disabled || plan.targets.length === 0} onclick={handleConfirm}>{plan.mode === 'trash' ? `Move to ${platformContextStore.trashLabel}` : 'Clean reviewed items'}</Button>
  </div>
</dialog>
