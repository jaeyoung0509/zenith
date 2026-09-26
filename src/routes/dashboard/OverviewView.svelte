<script lang="ts">
  import { onMount } from 'svelte';
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { memoryStore } from '../../lib/stores/memory.svelte';
  import { systemMetricsStore } from '../../lib/stores/systemMetrics.svelte';
  import { awakeStore } from '../../lib/stores/awake.svelte';
  import { dockerStore } from '../../lib/stores/docker.svelte';
  import { localModelsStore } from '../../lib/stores/models.svelte';
  import { developmentPortsStore } from '../../lib/stores/developmentPorts.svelte';
  import { usageStore } from '../../lib/stores/usage.svelte';
  import { agentActivityStore } from '../../lib/stores/agentActivity.svelte';
  import { platformCapabilitiesStore } from '../../lib/stores/platformCapabilities.svelte';
  import { formatBytes, formatCountdown, formatTimeAgo } from '../../lib/utils/format';
  import { cleanupSummaryState } from '../../lib/utils/cleanupSummary';
  import {
    batteryChargeStateLabel,
    memoryPressureLabel,
  } from '../../lib/utils/systemReadings';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import Button from '../../lib/components/Button.svelte';
  import ProgressBar from '../../lib/components/ProgressBar.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import MetricTile from '../../lib/components/metrics/MetricTile.svelte';
  import MetricSparkline from '../../lib/components/metrics/MetricSparkline.svelte';
  import MemoryPanel from '../../lib/components/performance/MemoryPanel.svelte';
  import ResourceRow from '../../lib/components/metrics/ResourceRow.svelte';
  import CelestialScene from '../../lib/components/CelestialScene.svelte';
  import {
    ChartNoAxesCombined,
    Boxes,
    Container,
    Gauge,
    HardDrive,
    Moon,
    RefreshCw,
    Server,
  } from '@lucide/svelte';

  interface Props {
    onNavigateTab?: (tab: string) => void;
  }

  let { onNavigateTab }: Props = $props();

  let reviewOpen = $state(false);
  let refreshing = $state(false);
  let disk = $derived(memoryStore.disk);

  async function refreshReadings() {
    if (refreshing) return;
    refreshing = true;
    try {
      await Promise.all([systemMetricsStore.refresh(), memoryStore.refresh()]);
    } finally { refreshing = false; }
  }

  function reviewResources() {
    reviewOpen = !reviewOpen;
    if (reviewOpen && memoryAvailable) void memoryStore.refreshMemory();
  }

  let settings = $derived(settingsStore.settings);
  let scan = $derived(scanStore.lastScan);
  let memory = $derived(memoryStore.memory);
  let cpu = $derived(systemMetricsStore.cpu);
  let battery = $derived(systemMetricsStore.battery);
  let awake = $derived(awakeStore.state);

  let cleanupAvailable = $derived(platformCapabilitiesStore.isAvailable('cleanup'));
  let memoryAvailable = $derived(platformCapabilitiesStore.isInspectable('memory_metrics'));
  let cpuAvailable = $derived(platformCapabilitiesStore.isInspectable('cpu_metrics'));
  let batteryCapability = $derived(platformCapabilitiesStore.feature('battery_metrics'));
  let awakeAvailable = $derived(platformCapabilitiesStore.isAvailable('keep_awake'));
  let dockerAvailable = $derived(platformCapabilitiesStore.isAvailable('docker'));
  let modelsAvailable = $derived(platformCapabilitiesStore.isAvailable('local_models'));
  let portsAvailable = $derived(platformCapabilitiesStore.isAvailable('development_ports'));
  let aiAvailable = $derived(platformCapabilitiesStore.isAvailable('ai_integrations'));

  let quickCleanableBytes = $derived(
    cleanupAvailable && scan ? scanStore.quickCleanableBytes(settings) : 0
  );
  let scannedAgo = $derived(scan ? formatTimeAgo(scan.finished_at) : null);
  let cleanupState = $derived(cleanupSummaryState({
    available: cleanupAvailable,
    hasScan: !!scan,
    scanning: scanStore.isScanning,
    cleaning: scanStore.isCleaning,
    freshness: scanStore.freshness,
    cleanableBytes: quickCleanableBytes,
  }));
  let cleanupBusy = $derived(
    cleanupState === 'scanning' || cleanupState === 'refreshing' || cleanupState === 'cleaning'
  );
  let cleanupActionLabel = $derived(
    cleanupState === 'cleaning'
      ? 'View cleanup'
      : cleanupBusy
        ? 'View scan'
        : cleanupState === 'ready' || (cleanupState === 'partial' && quickCleanableBytes > 0)
          ? 'Review'
          : 'Open Storage'
  );

  let cleanupValue = $derived.by(() => {
    switch (cleanupState) {
      case 'unavailable': return 'Unavailable';
      case 'unknown':
      case 'stale': return 'Scan needed';
      case 'failed': return 'Scan failed';
      case 'partial': return quickCleanableBytes > 0 ? formatBytes(quickCleanableBytes) : 'Partial scan';
      case 'ready':
      case 'clean': return formatBytes(quickCleanableBytes);
      default: return '';
    }
  });

  let cleanupDetail = $derived.by(() => {
    switch (cleanupState) {
      case 'unavailable': return platformCapabilitiesStore.feature('cleanup')?.reason ?? 'Cleanup is not available here.';
      case 'unknown': return 'No storage inventory has been measured yet.';
      case 'stale': return 'Scan again before reviewing cleanup.';
      case 'failed': return 'Scan could not finish. Open Storage for details.';
      case 'partial': return 'Some locations were not checked. Review measured items.';
      case 'ready': return 'Safe development and app caches Zenith can reclaim.';
      case 'clean': return 'Nothing verifiably cleanable in the last measured inventory.';
      default: return '';
    }
  });

  let cpuFreshness = $derived(
    cpu?.sampled_at != null ? formatTimeAgo(Math.floor(cpu.sampled_at / 1000)) : null
  );
  let memoryFreshness = $derived(
    memory?.timestamp != null ? formatTimeAgo(memory.timestamp) : null
  );
  let batteryFreshness = $derived(
    battery?.sampled_at != null ? formatTimeAgo(Math.floor(battery.sampled_at / 1000)) : null
  );

  let activeAgentCount = $derived(
    (agentActivityStore.snapshot?.unassigned_sessions.length ?? 0) +
      (agentActivityStore.snapshot?.projects.reduce((sum, project) => sum + project.sessions.length, 0) ?? 0)
  );
  let providerCount = $derived(
    usageStore.snapshot?.providers.filter((provider) => provider.connected).length ?? 0
  );
  let runningContainers = $derived(
    dockerStore.status?.containers.filter((container) => container.is_running).length ?? 0
  );

  /**
   * Overview composes the existing stores. It never mounts a full route to
   * collect data, and it releases the shared CPU/battery poller as soon as the
   * window is hidden.
   */
  onMount(() => {
    let stopFreshness: (() => void) | undefined;
    let polling = false;

    if (cleanupAvailable) {
      stopFreshness = scanStore.observeFreshness();
      void scanStore.init();
    }
    if (memoryAvailable) void memoryStore.refresh();
    if (awakeAvailable) void awakeStore.refresh();
    if (dockerAvailable) void dockerStore.refresh();
    if (modelsAvailable) void localModelsStore.refresh();
    if (portsAvailable) void developmentPortsStore.refresh();
    if (aiAvailable) {
      void usageStore.refreshIfStale();
      void agentActivityStore.refresh();
    }

    const startMetrics = () => {
      if (polling) return;
      if (!cpuAvailable && !batteryCapability && !memoryAvailable) return;
      polling = true;
      systemMetricsStore.startPolling(2500);
      if (memoryAvailable) memoryStore.startPolling(2500);
    };
    const stopMetrics = () => {
      if (!polling) return;
      polling = false;
      systemMetricsStore.stopPolling();
      if (memoryAvailable) memoryStore.stopPolling();
    };

    const onVisibilityChange = () => {
      if (document.visibilityState === 'hidden') stopMetrics();
      else startMetrics();
    };

    if (document.visibilityState !== 'hidden') startMetrics();
    document.addEventListener('visibilitychange', onVisibilityChange);

    return () => {
      document.removeEventListener('visibilitychange', onVisibilityChange);
      stopMetrics();
      stopFreshness?.();
    };
  });
</script>

<div class="overview-stage space-y-5">
  <PageHeader
    title="Overview"
    subtitle="Storage, running apps, and system activity in one place."
    icon={Gauge}
  >
    {#snippet actions()}
      <Button
        variant="ghost"
        size="sm"
        class="gap-1.5"
        ariaLabel="Refresh system readings"
        disabled={refreshing}
        onclick={() => void refreshReadings()}
      >
        {#if refreshing}<DeletingDots size="sm" />{:else}<RefreshCw size={14} aria-hidden="true" />{/if}
        <span>Refresh</span>
      </Button>
    {/snippet}
  </PageHeader>

  <!-- The next action, first. -->
  <section class="overview-hero flex flex-col items-start gap-3" aria-label="Cleanup summary">
    <div class="overview-celestial"><CelestialScene /></div>
    <div class="min-w-0 space-y-1">
      <div class="flex items-center gap-2">
        <HardDrive size={16} class="text-primary shrink-0" aria-hidden="true" />
        <span class="text-meta font-medium text-foreground">Cleanable storage</span>
        {#if scannedAgo && !cleanupBusy && cleanupState !== 'stale' && cleanupState !== 'failed'}
          <span class="text-caption font-mono text-muted-foreground">Scanned {scannedAgo}</span>
        {/if}
      </div>
      {#if cleanupBusy}
        <p class="flex min-h-8 items-center gap-2 text-body font-medium text-foreground" role="status" aria-live="polite">
          <DeletingDots size="sm" class="text-primary" />
          <span>{cleanupState === 'cleaning' ? 'Cleaning safe caches…' : scanStore.isRefreshingAfterClean ? 'Checking storage after cleanup…' : 'Checking storage…'}</span>
        </p>
      {:else}
        <p class="{cleanupState === 'ready' || cleanupState === 'clean' || (cleanupState === 'partial' && quickCleanableBytes > 0) ? 'overview-hero-value font-medium' : 'text-body font-semibold'} tabular-nums text-foreground">{cleanupValue}</p>
        <p class="text-meta text-muted-foreground break-words">{cleanupDetail}</p>
      {/if}
    </div>
    <div class="flex flex-wrap items-center gap-2">
      <Button variant="primary" size="md" onclick={reviewResources} ariaExpanded={reviewOpen} ariaControls="overview-resource-review" ariaLabel={reviewOpen ? "Close resource review" : "Review storage and running apps"}>
        {reviewOpen ? 'Close review' : 'Review resources'}
      </Button>
      {#if (cleanupState === 'ready' || cleanupState === 'partial') && scanStore.selectedCount > 0}
        <span class="text-meta text-muted-foreground whitespace-nowrap">
          {scanStore.selectedCount} selected · {formatBytes(scanStore.reclaimableBytes)}
        </span>
      {/if}
      <Button
        variant="secondary"
        size="md"
        disabled={!cleanupAvailable}
        onclick={() => onNavigateTab?.('storage')}
        ariaLabel={`${cleanupActionLabel} in Storage`}
        class="gap-1.5"
      >
        <span>{cleanupActionLabel}</span>
      </Button>
    </div>
  </section>

  <!-- Core readings. -->
  <section class="overview-readings grid gap-3 sm:grid-cols-3" aria-label="System readings">
    <MetricTile
      label="CPU"
      value={cpu?.usage_percent != null ? `${Math.round(cpu.usage_percent)}%` : '—'}
      valueClass={cpu?.usage_percent != null ? 'text-foreground' : 'text-muted-foreground'}
      freshness={cpuFreshness}
      detail={cpu && cpu.state !== 'fresh'
        ? cpu.state === 'stale'
          ? 'Last measured reading; sampling is paused.'
          : cpu.state === 'warmup'
            ? 'Waiting for a second reading.'
            : cpu.reason ?? 'CPU readings are not available on this platform.'
        : cpu
          ? `All ${cpu.cores} logical cores over ${(cpu.sample_interval_ms ?? 0) / 1000}s`
          : 'No CPU reading yet.'}
      actionLabel="Open Performance CPU detail"
      onclick={() => onNavigateTab?.('cpu')}
    >
      {#snippet visual()}
        {#if systemMetricsStore.cpuHistory.length > 0}
          <MetricSparkline samples={systemMetricsStore.cpuHistory} class="h-8" />
        {:else}
          <span class="text-caption text-muted-foreground">No history recorded yet</span>
        {/if}
      {/snippet}
    </MetricTile>

    <MetricTile
      label="Memory"
      value={memory ? formatBytes(memory.used_bytes) : '—'}
      tone={memory?.pressure === 'critical' ? 'critical' : memory?.pressure === 'warning' ? 'warning' : 'default'}
      freshness={memoryFreshness}
      detail={memory
        ? `${formatBytes(memory.total_bytes)} total${memory.swap_used_bytes > 0 ? ` · ${formatBytes(memory.swap_used_bytes)} swap` : ''}`
        : memoryStore.error ?? (memoryAvailable ? 'Reading memory…' : platformCapabilitiesStore.feature('memory_metrics')?.reason ?? 'Memory readings are not available on this platform.')}
      actionLabel="Open Performance memory detail"
      onclick={() => onNavigateTab?.('memory')}
    >
      {#snippet visual()}
        {#if memory && memory.total_bytes > 0}
          <span class="flex w-full flex-col gap-1.5">
            <span class="memory-pressure text-caption font-medium {memory.pressure === 'critical' ? 'text-destructive' : memory.pressure === 'warning' ? 'text-warning' : 'text-muted-foreground'}">
              <span class="memory-pressure-dot" aria-hidden="true"></span>
              {`${memoryPressureLabel(memory.pressure)} pressure`}
            </span>
            <ProgressBar
              value={(memory.used_bytes / memory.total_bytes) * 100}
              height="h-1.5"
              color={memory.pressure === 'critical' ? 'bg-destructive' : memory.pressure === 'warning' ? 'bg-warning' : 'bg-primary'}
            />
          </span>
        {/if}
      {/snippet}
    </MetricTile>

    <MetricTile
      label="Disk"
      value={disk ? `${Math.round(disk.percent_used ?? 0)}%` : '—'}
      detail={disk ? `${formatBytes(disk.available_bytes)} free of ${formatBytes(disk.total_bytes)}` : memoryStore.error ?? 'No disk reading yet.'}
      actionLabel="Open Storage disk details"
      onclick={() => onNavigateTab?.('disks')}
    >
      {#snippet visual()}
        {#if disk?.percent_used != null}
          <ProgressBar value={disk.percent_used} height="h-1.5" color="bg-primary" />
        {/if}
      {/snippet}
    </MetricTile>
  </section>

  {#if reviewOpen}
    <section id="overview-resource-review" class="resource-review space-y-4 rounded-xl border border-border bg-card p-4" aria-label="Resource review">
      <div class="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 class="text-sm font-semibold">Resource review</h2>
          <p class="mt-1 text-meta text-muted-foreground">Review disk caches, then quit apps you no longer need to reduce CPU and memory use.</p>
        </div>
        <Button variant="ghost" size="sm" onclick={() => (reviewOpen = false)}>Close review</Button>
      </div>
      <div class="flex flex-wrap items-center justify-between gap-3 rounded-lg bg-secondary/60 p-3">
        <div class="min-w-0">
          <p class="text-body font-medium">Storage · {cleanupBusy ? 'Checking…' : cleanupValue}</p>
          <p class="text-meta text-muted-foreground">{cleanupBusy ? 'Wait for the current operation to finish.' : cleanupDetail}</p>
        </div>
        <Button variant="primary" size="sm" disabled={!cleanupAvailable} onclick={() => onNavigateTab?.('storage')}>{cleanupActionLabel}</Button>
      </div>
      <div class="space-y-3">
        <h3 class="text-body font-semibold">Running apps</h3>
        {#if memoryAvailable}
          <MemoryPanel compact />
        {:else}
          <p class="text-meta text-muted-foreground">{platformCapabilitiesStore.feature('memory_metrics')?.reason ?? 'App memory readings are unavailable.'}</p>
        {/if}
      </div>
    </section>
  {/if}

  {#if battery?.presence === 'present'}
    <button type="button" class="flex w-full items-center justify-between gap-3 rounded-lg px-1 py-1 text-meta text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" onclick={() => onNavigateTab?.('battery')}>
      <span>Battery · {batteryChargeStateLabel(battery.charge_state)}</span>
      <span class="tabular-nums">{battery.percent != null ? `${Math.round(battery.percent)}%` : '—'} · {batteryFreshness ?? 'No recent reading'}</span>
    </button>
  {/if}

  <!-- Keep Awake, kept compact: state plus one action. -->
  <section class="rounded-xl border border-border bg-card p-3.5 flex flex-col sm:flex-row sm:items-center justify-between gap-3" aria-label="Keep Awake">
    <div class="flex items-center gap-2.5 min-w-0">
      <Moon size={16} class="text-muted-foreground shrink-0" aria-hidden="true" />
      <div class="min-w-0">
        <p class="text-body font-medium text-foreground">Keep Awake</p>
        <p class="text-meta text-muted-foreground truncate">
          {#if !awakeAvailable}
            {platformCapabilitiesStore.feature('keep_awake')?.reason ?? 'Keep Awake is unavailable on this platform.'}
          {:else if awake.is_active}
            {awake.active_process_name ?? (awake.trigger_source === 'manual' ? 'Manual session' : 'Rule matched')}
            {#if awake.manual_expires_at}
              · ends in {formatCountdown(Math.max(0, Math.floor(awake.manual_expires_at / 1000) - Math.floor(Date.now() / 1000)))}
            {/if}
          {:else}
            {awake.active_rules_count === 0 ? 'No rules enabled; sleep is allowed.' : `${awake.active_rules_count} enabled rule(s); nothing matched.`}
          {/if}
        </p>
      </div>
    </div>
    <div class="flex items-center gap-2 shrink-0">
      {#if awakeAvailable && awake.is_active}
        <Button variant="secondary" size="sm" onclick={() => void awakeStore.disableManual()} ariaLabel="Stop keeping this Mac awake">
          <span>Stop</span>
        </Button>
      {:else}
        <Button
          variant="secondary"
          size="sm"
          disabled={!awakeAvailable || awakeStore.isLoading}
          onclick={() => void awakeStore.setManual(3600, 'prevent_system_sleep')}
          ariaLabel="Keep this Mac awake for one hour"
        >
          <span>Keep awake 1 h</span>
        </Button>
      {/if}
      <Button variant="ghost" size="sm" onclick={() => onNavigateTab?.('awake')} ariaLabel="Open Keep Awake">
        <span>Open</span>
      </Button>
    </div>
  </section>

  <!-- Active tools and services. -->
  <section class="space-y-2" aria-label="Active tools and services">
    <h2 class="text-sm font-semibold tracking-tight text-foreground">Tools and services</h2>
    <div class="rounded-xl border border-border divide-y divide-border overflow-hidden">
      <ResourceRow
        label="Containers"
        value={dockerAvailable ? (dockerStore.status?.is_running ? `${runningContainers} running` : 'Stopped') : 'Unavailable'}
        tone={dockerAvailable && dockerStore.status?.is_running ? 'success' : 'muted'}
        detail={dockerAvailable
          ? dockerStore.status?.error_message ?? (dockerStore.status ? `${dockerStore.status.containers.length} containers · ${dockerStore.status.images.length} images` : null)
          : platformCapabilitiesStore.feature('docker')?.reason ?? null}
        onclick={() => onNavigateTab?.('docker')}
        actionLabel="Open Containers"
      >
        {#snippet icon()}<Container size={16} class="text-muted-foreground" aria-hidden="true" />{/snippet}
      </ResourceRow>

      <ResourceRow
        label="Local Models"
        value={modelsAvailable ? `${localModelsStore.models.length} installed` : 'Unavailable'}
        tone={modelsAvailable ? 'default' : 'muted'}
        detail={modelsAvailable
          ? localModelsStore.isUnavailable
            ? localModelsStore.incompleteReasons[0] ?? 'Inventory incomplete.'
            : `${formatBytes(localModelsStore.totalBytes)} on disk`
          : platformCapabilitiesStore.feature('local_models')?.reason ?? null}
        onclick={() => onNavigateTab?.('models')}
        actionLabel="Open Local Models"
      >
        {#snippet icon()}<Boxes size={16} class="text-muted-foreground" aria-hidden="true" />{/snippet}
      </ResourceRow>

      <ResourceRow
        label="Dev Servers"
        value={portsAvailable ? `${developmentPortsStore.listeners.length} listening` : 'Unavailable'}
        tone={portsAvailable && developmentPortsStore.listeners.length > 0 ? 'success' : 'muted'}
        detail={portsAvailable
          ? developmentPortsStore.listeners.length > 0
            ? developmentPortsStore.listeners
                .slice(0, 3)
                .map((listener) => `:${listener.port}`)
                .join(' · ')
            : 'Nothing is listening on a development port.'
          : platformCapabilitiesStore.feature('development_ports')?.reason ?? null}
        onclick={() => onNavigateTab?.('development_servers')}
        actionLabel="Open Dev Servers"
      >
        {#snippet icon()}<Server size={16} class="text-muted-foreground" aria-hidden="true" />{/snippet}
      </ResourceRow>

      <ResourceRow
        label="AI Activity"
        value={aiAvailable ? `${activeAgentCount} active` : 'Unavailable'}
        tone={activeAgentCount > 0 ? 'success' : 'muted'}
        detail={aiAvailable
          ? `${providerCount} connected ${providerCount === 1 ? 'account' : 'accounts'} · observed sessions only`
          : platformCapabilitiesStore.feature('ai_integrations')?.reason ?? null}
        onclick={() => onNavigateTab?.('projects')}
        actionLabel="Open AI Activity"
      >
        {#snippet icon()}
          <ChartNoAxesCombined size={16} class="text-muted-foreground" aria-hidden="true" />
        {/snippet}
      </ResourceRow>
    </div>
  </section>

  {#if !cleanupAvailable && !memoryAvailable && !awakeAvailable && !dockerAvailable && !modelsAvailable && !portsAvailable && !aiAvailable}
    <EmptyState
      title="No platform adapters are available"
      description="This platform has no implementing adapter for the inspected features, so Overview reports nothing instead of an empty success."
    />
  {/if}
</div>
