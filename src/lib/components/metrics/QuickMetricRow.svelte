<script lang="ts">
  import { ChevronRight, Cpu, MemoryStick, HardDrive, Battery, Gauge } from '@lucide/svelte';

  interface Props {
    label: string;
    value: string;
    detail?: string | null;
    actionLabel: string;
    onclick: () => void;
    tone?: 'default' | 'warning' | 'critical';
    meter?: number | null;
  }

  let {
    label,
    value,
    detail = null,
    actionLabel,
    onclick,
    tone = 'default',
    meter = null,
  }: Props = $props();
  let MetricIcon = $derived(label === 'CPU' ? Cpu : label === 'Memory' ? MemoryStick : label === 'Disk' ? HardDrive : label === 'Battery' ? Battery : Gauge);
  let meterValue = $derived(
    meter == null || !Number.isFinite(meter) ? null : Math.min(100, Math.max(0, meter))
  );
</script>

<button
  type="button"
  {onclick}
  aria-label={actionLabel}
  class="quick-data-row group flex w-full min-w-0 items-center gap-2.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
>
  <span class="quick-metric-icon flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-accent/60 text-primary" aria-hidden="true"><MetricIcon size={16} strokeWidth={1.75} /></span>
  <span class="min-w-0 flex-1">
    <span class="block text-meta font-medium text-foreground">{label}</span>
    {#if detail}
      <span class="block truncate text-caption text-muted-foreground" title={detail}>{detail}</span>
    {/if}
    {#if meterValue !== null}
      <span class="mt-1 block h-1 overflow-hidden rounded-full bg-secondary" aria-hidden="true">
        <span class="block h-full rounded-full bg-primary" style={`width: ${meterValue}%`}></span>
      </span>
    {/if}
  </span>
  <span class="quick-data-value shrink-0 text-right font-semibold tabular-nums {tone === 'critical' ? 'text-destructive' : tone === 'warning' ? 'text-warning' : 'text-foreground'}">{value}</span>
  <ChevronRight size={14} strokeWidth={1.75} class="shrink-0 text-muted-foreground" aria-hidden="true" />
</button>
