<script lang="ts">
  import { onMount } from 'svelte';
  import { FlaskConical } from '@lucide/svelte';
  import { apiBridgeStore } from '../stores/apiBridge.svelte';

  interface Props {
    /** Dot-and-tooltip form for the collapsed sidebar. */
    compact?: boolean;
    class?: string;
  }

  let { compact = false, class: className = '' }: Props = $props();

  // Server rendering has no webview, so the indicator would be noise there.
  const inBrowser = typeof window !== 'undefined';
  let snapshot = $derived(apiBridgeStore.snapshot);
  let visible = $derived(inBrowser && snapshot.mode === 'preview');
  let explanation = $derived(
    snapshot.message ??
      'Zenith is showing browser preview data instead of live results from this machine.'
  );

  onMount(() => apiBridgeStore.subscribe());
</script>

{#if visible}
  <div
    class="flex items-center gap-1.5 rounded-md border border-warning/25 bg-warning/10 px-1.5 py-1 text-caption font-medium text-warning {className}"
    role="status"
    title={explanation}
    data-testid="preview-mode-indicator"
  >
    <FlaskConical size={11} aria-hidden="true" class="shrink-0" />
    {#if !compact}
      <span class="truncate">Preview data</span>
    {/if}
    <span class="sr-only">{explanation}</span>
  </div>
{/if}
