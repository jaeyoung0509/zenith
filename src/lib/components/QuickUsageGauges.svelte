<script lang="ts">
  import type { UsageWindow } from '../models/types';
  import { formatTimeUntil } from '../utils/format';
  import ProgressBar from './ProgressBar.svelte';

  interface Props {
    windows: UsageWindow[];
    fallback: string;
  }

  let { windows, fallback }: Props = $props();

  let fiveHourWindow = $derived(
    windows.find((usageWindow) => {
      const label = usageWindow.label.toLowerCase();
      return label.includes('5h') || label.includes('5 hour');
    })
  );
  let weeklyWindow = $derived(
    windows.find((usageWindow) => usageWindow.label.toLowerCase().includes('week'))
  );

  function percent(usageWindow: UsageWindow): number {
    return Math.min(100, Math.max(0, Math.round(usageWindow.used_percent ?? 0)));
  }

  function compactReset(resetsAt: number | null): string {
    if (!resetsAt) return '';
    const timeUntil = formatTimeUntil(resetsAt);
    const match = timeUntil.match(/(\d+)([dhm])/);
    return match ? `${match[1]}${match[2]}` : timeUntil;
  }
</script>

{#if fiveHourWindow && weeklyWindow}
  <div class="grid w-48 shrink-0 grid-cols-2 gap-2" aria-label="Usage limit windows">
    {#each [
      { label: '5 hours', usageWindow: fiveHourWindow },
      { label: '1 week', usageWindow: weeklyWindow },
    ] as item (item.label)}
      <div
        class="min-w-0 space-y-1"
        role="meter"
        aria-label={`${item.label}: ${percent(item.usageWindow)}% used${compactReset(item.usageWindow.resets_at) ? `, resets in ${compactReset(item.usageWindow.resets_at)}` : ''}`}
        aria-valuemin="0"
        aria-valuemax="100"
        aria-valuenow={percent(item.usageWindow)}
      >
        <div class="flex items-baseline justify-between gap-1 font-mono text-micro">
          <span class="text-muted-foreground">{item.label}</span>
          <span class="shrink-0 text-foreground">
            {percent(item.usageWindow)}%
            {#if compactReset(item.usageWindow.resets_at)}
              <span class="text-muted-foreground"> · {compactReset(item.usageWindow.resets_at)}</span>
            {/if}
          </span>
        </div>
        <ProgressBar value={percent(item.usageWindow)} height="h-1.5" color="bg-violet-400" />
      </div>
    {/each}
  </div>
{:else}
  <span class="shrink-0 font-mono text-caption text-muted-foreground">{fallback}</span>
{/if}
