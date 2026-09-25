<script lang="ts">
  import type { CpuSample } from '../../stores/systemMetrics.svelte';

  interface Props {
    /** Real observations only. A gap in time is drawn as a gap, never interpolated. */
    samples: CpuSample[];
    /** Expected spacing between samples; a longer gap breaks the line. */
    expectedIntervalMs?: number;
    class?: string;
  }

  let { samples, expectedIntervalMs = 2500, class: className = '' }: Props = $props();

  const WIDTH = 100;
  const HEIGHT = 32;
  const GAP_FACTOR = 2.5;

  /** Consecutive samples drawn as one line; a time gap starts a new run. */
  let runs = $derived.by(() => {
    const result: CpuSample[][] = [];
    let current: CpuSample[] = [];
    for (const sample of samples) {
      const previous = current[current.length - 1];
      if (previous && sample.at - previous.at > expectedIntervalMs * GAP_FACTOR) {
        if (current.length > 0) result.push(current);
        current = [];
      }
      current.push(sample);
    }
    if (current.length > 0) result.push(current);
    return result;
  });

  let firstAt = $derived(samples[0]?.at ?? 0);
  let lastAt = $derived(samples[samples.length - 1]?.at ?? 0);

  /** The most recent reading sits at the right edge, which is where its value is read. */
  function pointX(at: number): number {
    const span = lastAt - firstAt;
    if (span <= 0) return WIDTH;
    return ((at - firstAt) / span) * WIDTH;
  }

  function pointY(percent: number): number {
    const clamped = Math.min(100, Math.max(0, percent));
    return HEIGHT - (clamped / 100) * HEIGHT;
  }

  function polyline(run: CpuSample[]): string {
    return run
      .map((sample) => `${pointX(sample.at).toFixed(1)},${pointY(sample.percent).toFixed(1)}`)
      .join(' ');
  }
</script>

<svg
  class="w-full {className}"
  viewBox="0 0 {WIDTH} {HEIGHT}"
  preserveAspectRatio="none"
  role="img"
  aria-label={samples.length > 0
    ? `CPU history with ${samples.length} recorded ${samples.length === 1 ? 'sample' : 'samples'}`
    : 'No CPU history recorded yet'}
>
  <line
    x1="0"
    y1={HEIGHT - 0.5}
    x2={WIDTH}
    y2={HEIGHT - 0.5}
    stroke="hsl(var(--border))"
    stroke-width="1"
    vector-effect="non-scaling-stroke"
  />
  {#each runs as run}
    {#if run.length === 1}
      <circle
        cx={pointX(run[0].at)}
        cy={pointY(run[0].percent)}
        r="1.5"
        fill="hsl(var(--primary))"
        vector-effect="non-scaling-stroke"
      />
    {:else}
      <polyline
        points={polyline(run)}
        fill="none"
        stroke="hsl(var(--primary))"
        stroke-width="1.5"
        stroke-linejoin="round"
        stroke-linecap="round"
        vector-effect="non-scaling-stroke"
      />
    {/if}
  {/each}
</svg>
