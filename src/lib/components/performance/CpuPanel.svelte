<script lang="ts">
  import type { CpuSampleState } from '../../models/types';
  import { systemMetricsStore } from '../../stores/systemMetrics.svelte';
  import { cpuStateDescription, cpuStateLabel } from '../../utils/systemReadings';
  import { formatTimeAgo } from '../../utils/format';
  import MetricSparkline from '../metrics/MetricSparkline.svelte';
  import Card from '../Card.svelte';
  import InlineNotice from '../InlineNotice.svelte';
  import { Cpu } from '@lucide/svelte';

  let cpu = $derived(systemMetricsStore.cpu);
  let history = $derived(systemMetricsStore.cpuHistory);

  /**
   * A missing reading with a failed probe is a failure, not a warm-up; every
   * other absent value is the backend's own warm-up state.
   */
  let state = $derived<CpuSampleState>(
    cpu?.state ?? (systemMetricsStore.cpuError ? 'failed' : 'warmup')
  );
  let percent = $derived(cpu?.usage_percent ?? null);
  let stateLabel = $derived(cpuStateLabel(state));
  let stateNote = $derived(cpuStateDescription(state, cpu?.reason ?? null));
  let sampledAgo = $derived(
    cpu?.sampled_at != null ? formatTimeAgo(Math.floor(cpu.sampled_at / 1000)) : null
  );
  let samplingWindow = $derived(
    cpu?.sample_interval_ms != null ? `${(cpu.sample_interval_ms / 1000).toFixed(1)} s` : 'Not measured yet'
  );
</script>

<div class="space-y-4">
  {#if systemMetricsStore.cpuError}
    <InlineNotice
      variant="destructive"
      title="CPU Reading Failed"
      message={systemMetricsStore.cpuError}
      onDismiss={() => (systemMetricsStore.cpuError = null)}
    />
  {/if}

  <Card class="space-y-4">
    <div class="flex items-center justify-between gap-2 text-meta font-medium text-muted-foreground">
      <span>System-wide CPU</span>
      <Cpu size={15} aria-hidden="true" />
    </div>

    <div class="flex min-h-16 flex-wrap items-center gap-x-3 gap-y-1">
      {#if percent != null}
        <!-- The value is only ever a measured pair of readings; the state stays beside it. -->
        <span class="text-metric font-mono font-semibold tabular-nums text-foreground">
          {percent.toFixed(1)}<span class="text-body font-normal text-muted-foreground">%</span>
        </span>
        <span class="rounded-full border border-border px-2 py-0.5 text-caption text-muted-foreground">{stateLabel}</span>
      {:else}
        <span class="text-metric font-semibold text-muted-foreground">{stateLabel}</span>
      {/if}
    </div>

    {#if state !== 'fresh'}
      <div class="rounded-lg border border-border bg-secondary/60 px-3 py-2.5 text-meta leading-relaxed text-muted-foreground">
        <span class="font-semibold text-foreground">{stateLabel}.</span>
        <span class="ml-1.5">{stateNote}</span>
      </div>
    {/if}

    <p class="text-meta text-muted-foreground">
      This is the share of all logical cores busy over one sampling window, so it is not the sum of per-process values.
    </p>

    <dl class="flex flex-wrap items-center gap-x-5 gap-y-1 text-meta text-muted-foreground">
      <div class="flex items-center gap-1.5">
        <dt>Logical cores</dt>
        <dd class="font-mono text-foreground">{cpu ? cpu.cores : '—'}</dd>
      </div>
      <div class="flex items-center gap-1.5">
        <dt>Sampling window</dt>
        <dd class="font-mono text-foreground">{samplingWindow}</dd>
      </div>
      <div class="flex items-center gap-1.5">
        <dt>Last reading</dt>
        <dd class="font-mono text-foreground">{sampledAgo ?? 'No reading yet'}</dd>
      </div>
    </dl>

    <div class="space-y-1.5 border-t border-border pt-3">
      <div class="flex items-center justify-between gap-2 text-meta text-muted-foreground">
        <span>Recorded history</span>
        <span class="font-mono">{history.length} {history.length === 1 ? 'sample' : 'samples'}</span>
      </div>
      <MetricSparkline
        samples={history}
        expectedIntervalMs={cpu?.sample_interval_ms ?? 2500}
        class="h-10"
      />
      <p class="text-caption text-muted-foreground">
        Samples appear as they were recorded; a gap means no reading was taken for that period.
      </p>
    </div>
  </Card>
</div>
