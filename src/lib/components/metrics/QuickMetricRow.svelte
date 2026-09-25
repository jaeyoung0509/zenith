<script lang="ts">
  import { ChevronRight } from '@lucide/svelte';

  interface Props {
    label: string;
    value: string;
    detail?: string | null;
    actionLabel: string;
    onclick: () => void;
    tone?: 'default' | 'warning' | 'critical';
  }

  let { label, value, detail = null, actionLabel, onclick, tone = 'default' }: Props = $props();
</script>

<button
  type="button"
  {onclick}
  aria-label={actionLabel}
  class="quick-data-row group flex w-full min-w-0 items-center gap-2.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
>
  <span class="min-w-0 flex-1">
    <span class="block text-meta font-medium text-foreground">{label}</span>
    {#if detail}
      <span class="block truncate text-caption text-muted-foreground" title={detail}>{detail}</span>
    {/if}
  </span>
  <span class="shrink-0 text-right text-body font-semibold tabular-nums {tone === 'critical' ? 'text-destructive' : tone === 'warning' ? 'text-warning' : 'text-foreground'}">{value}</span>
  <ChevronRight size={14} strokeWidth={1.75} class="shrink-0 text-muted-foreground" aria-hidden="true" />
</button>
