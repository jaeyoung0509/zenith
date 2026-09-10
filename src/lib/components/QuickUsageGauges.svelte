<script lang="ts">
  import type { UsageWindow } from '../models/types';
  import { formatTimeUntil } from '../utils/format';
  import { selectQuickUsageWindows } from '../utils/quickPanel';
  import ProgressBar from './ProgressBar.svelte';

  interface Props {
    windows: UsageWindow[];
    fallback: string;
  }

  let { windows, fallback }: Props = $props();

  let selectedWindows = $derived(selectQuickUsageWindows(windows));

  function percent(usageWindow: UsageWindow): number {
    return Math.min(100, Math.max(0, Math.round(usageWindow.used_percent ?? 0)));
  }

  function compactReset(resetsAt: number | null): string {
    if (!resetsAt) return '';
    const timeUntil = formatTimeUntil(resetsAt);
    const match = timeUntil.match(/(\d+)([dhm])/);
    return match ? `${match[1]}${match[2]}` : timeUntil;
  }

  let displayWindows = $derived.by(() => {
    if (!selectedWindows) return [];

    return [
      {
        label: '5 hours',
        usedPercent: percent(selectedWindows.fiveHour),
        reset: compactReset(selectedWindows.fiveHour.resets_at),
      },
      {
        label: '1 week',
        usedPercent: percent(selectedWindows.weekly),
        reset: compactReset(selectedWindows.weekly.resets_at),
      },
    ];
  });
</script>

{#if displayWindows.length === 2}
  <div
    class="grid w-full min-w-0 grid-cols-[repeat(auto-fit,minmax(min(8rem,100%),1fr))] gap-2"
    aria-label="Usage limit windows"
  >
    {#each displayWindows as item (item.label)}
      <div
        class="min-w-0 space-y-1"
        role="meter"
        aria-label={`${item.label}: ${item.usedPercent}% used${item.reset ? `, resets in ${item.reset}` : ''}`}
        aria-valuemin="0"
        aria-valuemax="100"
        aria-valuenow={item.usedPercent}
      >
        <div class="flex min-w-0 items-baseline justify-between gap-2 whitespace-nowrap font-mono text-micro">
          <span class="shrink-0 text-muted-foreground">{item.label}</span>
          <span class="shrink-0 text-right tabular-nums text-foreground">
            {item.usedPercent}%
            {#if item.reset}
              <span class="whitespace-nowrap text-muted-foreground"> · {item.reset}</span>
            {/if}
          </span>
        </div>
        <ProgressBar value={item.usedPercent} height="h-1.5" color="bg-violet-400" />
      </div>
    {/each}
  </div>
{:else}
  <span class="shrink-0 whitespace-nowrap font-mono tabular-nums text-caption text-muted-foreground">{fallback}</span>
{/if}
