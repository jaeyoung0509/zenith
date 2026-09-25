<script lang="ts">
  import type { PowerSourceType } from '../../models/types';
  import { systemMetricsStore } from '../../stores/systemMetrics.svelte';
  import {
    batteryChargeStateLabel,
    batteryPresenceLabel,
    formatRemainingTime,
  } from '../../utils/systemReadings';
  import { formatTimeAgo } from '../../utils/format';
  import BatteryIndicator from '../metrics/BatteryIndicator.svelte';
  import Card from '../Card.svelte';
  import EmptyState from '../EmptyState.svelte';
  import InlineNotice from '../InlineNotice.svelte';
  import { Battery } from '@lucide/svelte';

  /** The backend's tri-state power source, in the platform's own words. */
  const POWER_SOURCE_LABELS: Record<PowerSourceType, string> = {
    ac: 'AC power',
    battery: 'Battery',
    unknown: 'Not reported',
  };

  let battery = $derived(systemMetricsStore.battery);
  let percent = $derived(battery?.percent ?? null);
  // Absent, zero, and "unlimited" all stay absent: only a real estimate is shown.
  let remaining = $derived(formatRemainingTime(battery?.time_remaining_seconds ?? null));
  let sampledAgo = $derived(
    battery?.sampled_at != null ? formatTimeAgo(Math.floor(battery.sampled_at / 1000)) : null
  );
</script>

<div class="space-y-4">
  {#if systemMetricsStore.batteryError}
    <InlineNotice
      variant="destructive"
      title="Battery Reading Failed"
      message={systemMetricsStore.batteryError}
      onDismiss={() => (systemMetricsStore.batteryError = null)}
    />
  {/if}

  {#if !battery}
    <div class="space-y-2 py-12 text-center text-meta text-muted-foreground">
      <Battery size={20} class="mx-auto opacity-50" aria-hidden="true" />
      <p>Reading the battery state...</p>
    </div>
  {:else if battery.presence === 'absent'}
    <EmptyState
      icon={Battery}
      title="No battery"
      description="This machine reports no internal battery."
    />
  {:else if battery.presence === 'unavailable'}
    <EmptyState
      icon={Battery}
      title={batteryPresenceLabel('unavailable')}
      description={battery.reason ?? 'The platform did not return a battery reading.'}
    />
  {:else}
    <Card class="space-y-4">
      <div class="flex flex-wrap items-center justify-between gap-2 text-meta font-medium text-muted-foreground">
        <span>{batteryPresenceLabel(battery.presence)}</span>
        {#if sampledAgo}
          <span class="font-mono text-caption">Updated {sampledAgo}</span>
        {/if}
      </div>

      <div class="flex min-h-16 items-center gap-3">
        <BatteryIndicator percent={percent} chargeState={battery.charge_state} />
        <div class="min-w-0 space-y-0.5">
          {#if percent != null}
            <div class="text-metric font-mono font-semibold tabular-nums text-foreground">
              {Math.round(percent)}<span class="text-body font-normal text-muted-foreground">%</span>
            </div>
          {:else}
            <div class="text-body font-medium text-muted-foreground">Charge level unavailable</div>
          {/if}
          <div class="text-meta text-muted-foreground">{batteryChargeStateLabel(battery.charge_state)}</div>
        </div>
      </div>

      {#if remaining}
        <!-- An estimate is attributed to the platform that produced it. -->
        <p class="text-meta text-muted-foreground">Platform estimate: {remaining}.</p>
      {/if}

      <dl class="flex flex-wrap items-center gap-x-5 gap-y-1 border-t border-border pt-3 text-meta text-muted-foreground">
        <div class="flex items-center gap-1.5">
          <dt>Power source</dt>
          <dd class="text-foreground">{POWER_SOURCE_LABELS[battery.power_source]}</dd>
        </div>
        <div class="flex items-center gap-1.5">
          <dt>Last reading</dt>
          <dd class="font-mono text-foreground">{sampledAgo ?? 'Not reported'}</dd>
        </div>
      </dl>
    </Card>
  {/if}
</div>
