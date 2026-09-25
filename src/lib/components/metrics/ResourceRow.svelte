<script lang="ts">
  import type { Snippet } from 'svelte';
  import { ChevronRight } from '@lucide/svelte';

  interface Props {
    label: string;
    /** Observed state, in the domain's own words. */
    value: string;
    /** One supporting fact; kept separate from the name column. */
    detail?: string | null;
    tone?: 'default' | 'success' | 'warning' | 'critical' | 'muted';
    onclick?: () => void;
    actionLabel?: string;
    icon?: Snippet;
    class?: string;
  }

  let {
    label,
    value,
    detail = null,
    tone = 'default',
    onclick,
    actionLabel,
    icon,
    class: className = '',
  }: Props = $props();

  const toneClass = {
    default: 'text-foreground',
    success: 'text-success',
    warning: 'text-warning',
    critical: 'text-destructive',
    muted: 'text-muted-foreground',
  };
</script>

{#if onclick}
  <button
    type="button"
    onclick={onclick}
    aria-label={actionLabel ?? `${label}: ${value}`}
    class="group w-full min-h-11 flex items-center gap-3 px-3 py-2 text-left transition-[background-color] duration-140 hover:bg-accent/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset {className}"
  >
    {#if icon}
      <span class="shrink-0 flex items-center justify-center w-5">{@render icon()}</span>
    {/if}
    <span class="min-w-0 flex-1 text-body font-medium text-foreground truncate">{label}</span>
    {#if detail}
      <span class="hidden sm:block min-w-0 max-w-[45%] truncate text-meta text-muted-foreground">{detail}</span>
    {/if}
    <span class="shrink-0 text-meta font-medium whitespace-nowrap {toneClass[tone]}">{value}</span>
    <ChevronRight size={14} class="shrink-0 text-muted-foreground" aria-hidden="true" />
  </button>
{:else}
  <div class="w-full min-h-11 flex items-center gap-3 px-3 py-2 {className}">
    {#if icon}
      <span class="shrink-0 flex items-center justify-center w-5">{@render icon()}</span>
    {/if}
    <span class="min-w-0 flex-1 text-body font-medium text-foreground truncate">{label}</span>
    {#if detail}
      <span class="hidden sm:block min-w-0 max-w-[45%] truncate text-meta text-muted-foreground">{detail}</span>
    {/if}
    <span class="shrink-0 text-meta font-medium whitespace-nowrap {toneClass[tone]}">{value}</span>
  </div>
{/if}
