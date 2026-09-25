<script lang="ts">
  import type { Snippet } from 'svelte';
  import { ChevronRight } from '@lucide/svelte';

  interface Props {
    label: string;
    /** The current reading, or a named state when there is no reading. */
    value: string;
    valueClass?: string;
    /** Supporting fact; never a duplicated verdict. */
    detail?: string | null;
    /** When the reading was taken, if the backend gave a timestamp. */
    freshness?: string | null;
    tone?: 'default' | 'warning' | 'critical';
    onclick?: () => void;
    /** Accessible name for the whole tile when it navigates. */
    actionLabel?: string;
    /** Visual for the reading: a sparkline, a battery outline, a bar. */
    visual?: Snippet;
    class?: string;
  }

  let {
    label,
    value,
    valueClass = 'text-foreground',
    detail = null,
    freshness = null,
    tone = 'default',
    onclick,
    actionLabel,
    visual,
    class: className = '',
  }: Props = $props();

  const toneClass = {
    default: '',
    warning: 'border-warning/40',
    critical: 'border-destructive/40',
  };
</script>

<svelte:element
  this={onclick ? 'button' : 'div'}
  type={onclick ? 'button' : undefined}
  role={onclick ? 'button' : undefined}
  onclick={onclick}
  aria-label={onclick ? actionLabel ?? label : undefined}
  class="metric-tile group flex flex-col min-w-0 w-full text-left rounded-xl border border-border bg-card shadow-sm p-3.5 space-y-1.5 {onclick
    ? 'transition-[background-color,border-color] duration-140 hover:border-border-strong hover:bg-accent/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring'
    : ''} {toneClass[tone]} {className}"
>
  <span class="flex items-center justify-between gap-2">
    <span class="text-meta font-medium text-muted-foreground">{label}</span>
    <span class="flex items-center gap-1.5">
      {#if freshness}
        <span class="text-caption font-mono text-muted-foreground whitespace-nowrap">{freshness}</span>
      {/if}
      {#if onclick}
        <ChevronRight size={14} class="text-muted-foreground" aria-hidden="true" />
      {/if}
    </span>
  </span>
  <span class="block text-metric font-sans tabular-nums font-medium tracking-tight whitespace-nowrap {valueClass}">{value}</span>
  {#if visual}
    <span class="flex h-8 shrink-0 items-center">
      {@render visual()}
    </span>
  {/if}
  {#if detail}
    <span class="block text-meta text-muted-foreground [overflow-wrap:normal] break-words">{detail}</span>
  {/if}
</svelte:element>
