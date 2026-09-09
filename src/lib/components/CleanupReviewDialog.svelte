<script lang="ts">
  import { onMount } from 'svelte';
  import type { ScanItem } from '../models/types';
  import { formatBytes } from '../utils/format';
  import Button from './Button.svelte';
  import RiskBadge from './RiskBadge.svelte';

  let { items, disabled = false, onCancel, onConfirm }: {
    items: ScanItem[];
    disabled?: boolean;
    onCancel: () => void;
    onConfirm: () => void;
  } = $props();
  const id = $props.id();
  let dialog: HTMLDialogElement;
  let hasRebuild = $derived(items.some(item => item.risk === 'rebuild'));

  onMount(() => {
    const previousFocus = document.activeElement;
    dialog.showModal();
    return () => {
      dialog.close();
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
    };
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby={id + '-title'}
  aria-describedby={id + '-description'}
  oncancel={(event) => { event.preventDefault(); onCancel(); }}
  class="m-auto w-[calc(100%-2rem)] max-w-lg max-h-[calc(100%-2rem)] overflow-y-auto rounded-xl border border-border bg-card p-5 text-foreground shadow-xl backdrop:bg-black/50"
>
  <h2 id={id + '-title'} class="text-base font-semibold">Review cleanup</h2>
  <p id={id + '-description'} class="mt-2 text-sm text-muted-foreground">
    Review these {items.length} selected items before continuing. Cache cleanup cannot be undone in Zenith.
  </p>
  {#if hasRebuild}
    <p class="mt-2 text-sm text-warning">Rebuild items may require downloads or recompilation the next time you use the tool.</p>
  {/if}
  <ul class="my-4 divide-y divide-border">
    {#each items as item (item.id)}
      <li class="flex flex-wrap items-center justify-between gap-2 py-2 text-sm">
        <span class="min-w-0 break-words">{item.name}</span>
        <span class="flex items-center gap-2"><RiskBadge risk={item.risk} /><span class="font-mono whitespace-nowrap">{formatBytes(item.size.allocated ?? item.size.logical)}</span></span>
      </li>
    {/each}
  </ul>
  {#if disabled}
    <p role="status" class="mb-3 text-sm text-warning">The scan changed or expired. Cancel and review a fresh selection.</p>
  {/if}
  <div class="flex flex-wrap justify-end gap-2">
    <Button variant="secondary" onclick={onCancel}>Cancel</Button>
    <Button variant="destructive" disabled={disabled || items.length === 0} onclick={onConfirm}>Clean reviewed items</Button>
  </div>
</dialog>
