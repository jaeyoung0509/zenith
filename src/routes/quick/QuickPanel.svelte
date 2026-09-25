<script lang="ts">
  import { onMount } from 'svelte';
  import type {
    DashboardRoute,
    AgentQuickSummary,
    QuickPanelSection,
  } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { memoryStore } from '../../lib/stores/memory.svelte';
  import { systemMetricsStore } from '../../lib/stores/systemMetrics.svelte';
  import { awakeStore } from '../../lib/stores/awake.svelte';
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import { platformCapabilitiesStore } from '../../lib/stores/platformCapabilities.svelte';
  import { platformContextStore } from '../../lib/stores/platformContext.svelte';
  import { usageStore } from '../../lib/stores/usage.svelte';
  import { formatBytes, formatTimeAgo, formatTimeUntil } from '../../lib/utils/format';
  import { batteryChargeStateLabel, memoryPressureLabel } from '../../lib/utils/systemReadings';
  import {
    handleQuickPanelFocusChanged,
    isAcceleratorPressed,
    isQuickPanelDismissShortcut,
    platformAccelerator,
    projectAiProviders,
    projectQuickAiRows,
    formatQuickProviderUsage,
  } from '../../lib/utils/quickPanel';
  import {
    isTauri,
    tauriHideCurrentWindow,
    tauriGetAgentQuickSummary,
    tauriOpenDashboard,
    tauriStartWindowDrag,
  } from '../../lib/utils/tauri';
  import { APP_VERSION, formatVersion } from '../../lib/utils/version';
  import Button from '../../lib/components/Button.svelte';
  import BrandIcon from '../../lib/components/BrandIcon.svelte';
  import QuickMetricRow from '../../lib/components/metrics/QuickMetricRow.svelte';
  import CleanResultModal from '../../lib/components/CleanResultModal.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import LoadingSpinner from '../../lib/components/LoadingSpinner.svelte';
  import InlineNotice from '../../lib/components/InlineNotice.svelte';
  import PreviewModeIndicator from '../../lib/components/PreviewModeIndicator.svelte';
  import {
    ArrowRight,
    Moon,
    RefreshCw,
    Settings,
    Trash2,
    X,
  } from '@lucide/svelte';

  /** Keep the panel useful without turning it into an agent inventory. */
  const AI_ROW_LIMIT = 4;

  let panelActive = false;
  let showResultModal = $state(false);
  let agentSummary = $state<AgentQuickSummary | null>(null);
  let settings = $derived(settingsStore.settings);
  let disk = $derived(memoryStore.disk);
  let memory = $derived(memoryStore.memory);
  let cpu = $derived(systemMetricsStore.cpu);
  let battery = $derived(systemMetricsStore.battery);
  let scan = $derived(scanStore.lastScan);
  let awakeState = $derived(awakeStore.state);
  let selectedProviders = $derived(
    projectAiProviders(
      settings.quick_panel_ai_providers,
      usageStore.snapshot?.providers,
      usageStore.isLoading
    )
  );
  let cleanupCapability = $derived(platformCapabilitiesStore.feature('cleanup'));
  let awakeCapability = $derived(platformCapabilitiesStore.feature('keep_awake'));
  let aiCapability = $derived(platformCapabilitiesStore.feature('ai_integrations'));
  let cpuCapability = $derived(platformCapabilitiesStore.feature('cpu_metrics'));
  let batteryCapability = $derived(platformCapabilitiesStore.feature('battery_metrics'));
  let cleanupAvailable = $derived(cleanupCapability?.status === 'available');
  let memoryAvailable = $derived(platformCapabilitiesStore.isInspectable('memory_metrics'));
  let cpuAvailable = $derived(cpuCapability?.status === 'available' || cpuCapability?.status === 'read_only');
  let awakeAvailable = $derived(awakeCapability?.status === 'available');
  let aiAvailable = $derived(aiCapability?.status === 'available');
  // A capability query failure is not an unsupported platform: the quick panel
  // must offer a retry instead of claiming the feature does not exist.
  let capabilitiesFailed = $derived(
    platformCapabilitiesStore.error !== null && platformCapabilitiesStore.capabilities === null
  );

  let quickCleanableBytes = $derived.by(() =>
    scanStore.quickCleanableBytes(settings)
  );

  let cleanupState = $derived.by(() => {
    if (!scan) {
      return scanStore.isScanning ? 'scanning' : 'unknown';
    }
    if (scanStore.isScanning) {
      return 'refreshing';
    }
    if (scanStore.freshness !== 'fresh') return 'stale';
    return quickCleanableBytes > 0 ? 'ready' : 'clean';
  });

  let cleanupValue = $derived.by(() => {
    if (cleanupState === 'unknown') return '—';
    if (cleanupState === 'scanning') return 'Scanning…';
    return formatBytes(quickCleanableBytes);
  });

  let cleanupDetail = $derived.by(() => {
    switch (cleanupState) {
      case 'unknown':
        return 'Run a scan to check safe caches.';
      case 'scanning':
        return 'Checking development caches.';
      case 'refreshing':
        return scanStore.lastScanTrigger === 'auto' ? 'Auto-refreshing…' : 'Refreshing scan…';
      case 'stale':
        return 'Scan again to verify safe cleanup.';
      case 'ready':
        return 'Safe development and app caches.';
      case 'clean':
        return 'Nothing verifiably cleanable was measured.';
    }
  });


  function hasSection(section: QuickPanelSection) {
    return settings.quick_panel_sections.includes(section);
  }

  let aiRows = $derived(projectQuickAiRows(selectedProviders, agentSummary?.sessions ?? []));
  let visibleAiRows = $derived(aiRows.slice(0, AI_ROW_LIMIT));
  let hiddenAiCount = $derived(Math.max(0, aiRows.length - AI_ROW_LIMIT));
  let activeCount = $derived(agentSummary?.active_count ?? 0);

  let stopFreshness: (() => void) | undefined;
  let metricsPolling = false;

  async function activatePanel() {
    if (panelActive) return;
    panelActive = true;
    await refreshPanelData();
  }

  /** Runs (or re-runs, after a capability failure) the panel's data loads. */
  async function refreshPanelData() {
    await settingsStore.load(true);
    await platformCapabilitiesStore.load(true);
    await platformContextStore.load(true);
    if (!panelActive) return;
    if (awakeAvailable) void awakeStore.refresh();
    if (hasSection('storage') && cleanupAvailable) void memoryStore.refreshDisk();
    if (hasSection('memory') && memoryAvailable) memoryStore.startPolling(3000);
    if ((hasSection('cpu') && cpuAvailable) || hasSection('battery')) {
      metricsPolling = true;
      systemMetricsStore.startPolling(3000, 30_000);
    }
    if (
      (hasSection('agent_activity') || hasSection('categories')) &&
      aiAvailable &&
      settings.quick_panel_ai_providers.length > 0
    ) {
      void usageStore.refreshIfStale();
    }
    if (hasSection('agent_activity') && aiAvailable) {
      void tauriGetAgentQuickSummary().then((summary) => {
        if (panelActive) agentSummary = summary;
      });
    }
    if ((hasSection('cleanup') || hasSection('categories')) && cleanupAvailable) {
      stopFreshness?.();
      stopFreshness = scanStore.observeFreshness();
      await scanStore.init();
      if (panelActive && scanStore.isStale()) void scanStore.runScan();
    }
  }

  function deactivatePanel() {
    if (!panelActive) return;
    panelActive = false;
    stopFreshness?.();
    stopFreshness = undefined;
    if (hasSection('memory')) memoryStore.stopPolling();
    if (metricsPolling) {
      metricsPolling = false;
      systemMetricsStore.stopPolling();
    }
  }

  onMount(() => {
    let disposed = false;
    let unlistenFocus: (() => void) | undefined;

    const closeOnShortcut = (event: KeyboardEvent) => {
      const accelerator = platformAccelerator(platformContextStore.context?.primary_accelerator);
      if (
        isQuickPanelDismissShortcut(event.key, isAcceleratorPressed(event, accelerator))
      ) {
        event.preventDefault();
        event.stopImmediatePropagation();
        handleClose();
      }
    };
    window.addEventListener('keydown', closeOnShortcut, true);

    const onVisibilityChange = () => {
      if (document.visibilityState === 'hidden') deactivatePanel();
    };
    document.addEventListener('visibilitychange', onVisibilityChange);

    if (!isTauri()) {
      void activatePanel();
    } else {
      void import('@tauri-apps/api/webviewWindow').then(async ({ getCurrentWebviewWindow }) => {
        const currentWindow = getCurrentWebviewWindow();
        unlistenFocus = await currentWindow.onFocusChanged(({ payload: focused }) => {
          handleQuickPanelFocusChanged(focused, {
            activate: activatePanel,
            deactivate: deactivatePanel,
          });
        });
        if (!disposed && await currentWindow.isVisible()) void activatePanel();
      });
    }

    return () => {
      disposed = true;
      unlistenFocus?.();
      window.removeEventListener('keydown', closeOnShortcut, true);
      document.removeEventListener('visibilitychange', onVisibilityChange);
      deactivatePanel();
    };
  });

  async function handleCleanSafe() {
    const result = await scanStore.quickCleanSafe();
    if (result) {
      showResultModal = true;
    }
  }

  function handleOpenDashboard() {
    deactivatePanel();
    void tauriOpenDashboard();
  }

  function handleOpenRoute(route: DashboardRoute) {
    deactivatePanel();
    void tauriOpenDashboard(route);
  }

  function handleClose() {
    deactivatePanel();
    void tauriHideCurrentWindow();
  }

  /** Retries a failed capability query without reloading the whole panel. */
  async function retryCapabilities() {
    await platformCapabilitiesStore.load(true);
    if (platformCapabilitiesStore.capabilities) await refreshPanelData();
  }

  function handleWindowDrag(event: MouseEvent) {
    if (event.button !== 0) return;
    const target = event.target;
    if (target instanceof Element && target.closest('.no-drag')) return;
    void tauriStartWindowDrag().catch(() => undefined);
  }

  let awakeRemaining = $derived.by(() => {
    if (!awakeState.manual_expires_at) return null;
    return formatTimeUntil(Math.floor(awakeState.manual_expires_at / 1000));
  });
</script>

<div class="quick-liquid-shell w-full h-full rounded-2xl flex flex-col select-none text-foreground font-sans overflow-hidden relative">
  <!-- Header — draggable, buttons are no-drag -->
  <div
    class="quick-liquid-chrome flex shrink-0 items-center justify-between gap-2 px-4 py-3 border-b border-border relative titlebar-drag-region"
    role="presentation"
    onmousedown={handleWindowDrag}
  >
    <div class="flex items-center gap-2 min-w-0">
      <BrandIcon identity="zenith" label="Zenith" size={20} />
      <span class="text-body font-semibold tracking-tight truncate">Zenith</span>
    </div>
    <div class="flex items-center gap-1 no-drag shrink-0">
      <Button
        variant="ghost"
        size="icon"
        class="h-7 w-7 text-muted-foreground hover:text-foreground hover:bg-accent"
        onclick={() => handleOpenRoute('settings')}
        ariaLabel="Open Settings"
        title="Open Settings"
      >
        <Settings size={15} aria-hidden="true" />
      </Button>
      <Button
        variant="ghost"
        size="icon"
        class="h-7 w-7 text-muted-foreground hover:text-foreground hover:bg-accent"
        onclick={handleClose}
        ariaLabel="Close quick panel"
        title="Close quick panel (Esc)"
      >
        <X size={15} aria-hidden="true" />
      </Button>
    </div>
  </div>

  <!-- Body Content — the panel's single scrolling region -->
  <div class="min-h-0 flex-1 overflow-y-auto scroll-stable px-3 py-2">
    {#if capabilitiesFailed}
      <!-- A failed capability query is not an unsupported platform: offer a retry
           instead of reporting missing features. -->
      <InlineNotice
        variant="error"
        title="Platform capabilities unavailable"
        message={platformCapabilitiesStore.error ?? 'Zenith could not read this platform\'s capability matrix from the backend.'}
        actionLabel="Retry"
        onAction={() => void retryCapabilities()}
      />
    {:else}
      {#each settings.quick_panel_sections as section (section)}
        {@render sectionCell(section)}
      {/each}
    {/if}
  </div>

  <!-- Footer -->
  <div class="quick-liquid-chrome shrink-0 pt-3 border-t px-4 pb-3 flex items-center justify-between gap-2">
    <div class="flex items-center gap-1.5 text-caption text-muted-foreground min-w-0">
      <button
        id="quick-storage-scan-button"
        type="button"
        disabled={!cleanupAvailable || scanStore.isScanning || scanStore.isCleaning}
        onclick={() => {
          if (cleanupAvailable) void scanStore.runScan();
        }}
        class="p-1 rounded-md hover:bg-accent text-muted-foreground hover:text-foreground transition-colors duration-140 disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        aria-label="Refresh storage scan"
        title={cleanupAvailable ? 'Rescan storage' : (cleanupCapability?.reason ?? 'Storage cleanup is unavailable on this platform.')}
      >
        <span class="inline-flex items-center justify-center shrink-0 w-3.5 h-3.5">
          {#if scanStore.isScanning}
            <LoadingSpinner size={12} />
          {:else}
            <RefreshCw size={12} aria-hidden="true" />
          {/if}
        </span>
      </button>
    </div>
    <div class="flex items-center gap-1.5 shrink-0">
      <PreviewModeIndicator />
      <span class="quick-version text-caption font-mono text-muted-foreground select-none">{formatVersion(APP_VERSION)}</span>
      <Button
        variant="secondary"
        size="sm"
        onclick={handleOpenDashboard}
        class="gap-1.5 text-meta"
      >
        <span>Open Zenith</span>
        <ArrowRight size={13} aria-hidden="true" />
      </Button>
    </div>
  </div>

  {#if showResultModal && scanStore.lastCleanResult}
    <CleanResultModal
      result={scanStore.lastCleanResult}
      onClose={() => (showResultModal = false)}
      returnFocusTargetId="quick-storage-scan-button"
    />
  {/if}
</div>

{#snippet sectionCell(section: QuickPanelSection)}
  {#if section === 'cleanup'}
    <section class="quick-list-section" aria-label="Cleanup">
      <div class="flex items-start justify-between gap-2">
        <div class="min-w-0">
          <p class="text-meta font-semibold text-foreground">Cleanup <span class="ml-1 font-mono tabular-nums text-muted-foreground">{cleanupValue}</span></p>
          <p class="text-caption text-muted-foreground [overflow-wrap:normal] break-words">{cleanupDetail}</p>
        </div>
        <Button
          variant="ghost"
          size="sm"
          class="gap-1 shrink-0 text-meta"
          disabled={!cleanupAvailable}
          onclick={() => handleOpenRoute('storage')}
          ariaLabel="Review cleanup in the main window"
        >
          <span>Review</span>
          <ArrowRight size={12} aria-hidden="true" />
        </Button>
      </div>
      {#if cleanupAvailable && (cleanupState === 'ready' || cleanupState === 'stale' || scanStore.isCleaning)}
        <div class="mt-2 flex items-center gap-2">
          {#if cleanupState === 'stale'}
            <Button
              variant="secondary"
              size="sm"
              disabled={scanStore.isScanning || scanStore.isCleaning}
              onclick={() => void scanStore.runScan()}
              class="gap-1.5 text-meta"
            >
              <RefreshCw size={13} aria-hidden="true" />
              <span>Scan Again</span>
            </Button>
          {:else}
            <Button
              variant="primary"
              size="sm"
              disabled={cleanupState !== 'ready' || !scanStore.canClean}
              onclick={handleCleanSafe}
              class="gap-1.5 text-meta"
            >
              {#if scanStore.isCleaning}
                <DeletingDots size="xs" />
                <span>Cleaning</span>
              {:else}
                <Trash2 size={13} aria-hidden="true" />
                <span>Clean Safe</span>
              {/if}
            </Button>
          {/if}
        </div>
      {/if}
    </section>
  {:else if section === 'cpu'}
    <QuickMetricRow
      label="CPU"
      value={cpu?.usage_percent != null ? `${Math.round(cpu.usage_percent)}%` : cpuAvailable ? 'Warming up' : 'Unavailable'}
      detail={cpu && cpu.state !== 'fresh'
        ? cpu.state === 'stale'
          ? 'Last reading · paused'
          : cpu.state === 'warmup'
            ? 'Waiting for a second reading'
            : cpu.reason ?? cpuCapability?.reason ?? 'No CPU adapter here.'
        : cpu
          ? `All ${cpu.cores} cores · ${cpu.sampled_at != null ? formatTimeAgo(Math.floor(cpu.sampled_at / 1000)) : 'Reading'}`
          : cpuCapability?.reason ?? null}
      actionLabel="Open CPU detail"
      onclick={() => handleOpenRoute('cpu')}
    />
  {:else if section === 'memory'}
    <QuickMetricRow
      label="Memory"
      value={memory ? memoryPressureLabel(memory.pressure) : memoryAvailable ? 'Reading…' : 'Unavailable'}
      tone={memory?.pressure === 'critical' ? 'critical' : memory?.pressure === 'warning' ? 'warning' : 'default'}
      detail={memory
        ? `${formatBytes(memory.used_bytes)} of ${formatBytes(memory.total_bytes)}`
        : platformCapabilitiesStore.feature('memory_metrics')?.reason ?? null}
      actionLabel="Open memory detail"
      onclick={() => handleOpenRoute('memory')}
    />
  {:else if section === 'battery'}
    {@const batteryPresent = battery?.presence === 'present'}
    {#if !battery || batteryPresent || battery.presence === 'unavailable'}
      <QuickMetricRow
        label="Battery"
        value={batteryPresent && battery?.percent != null ? `${Math.round(battery.percent)}%` : battery ? batteryChargeStateLabel(battery.charge_state) : 'Reading…'}
        detail={batteryPresent
          ? batteryChargeStateLabel(battery!.charge_state)
          : battery?.reason ?? batteryCapability?.reason ?? 'No battery reported.'}
        actionLabel="Open battery detail"
        onclick={() => handleOpenRoute('battery')}
      />
    {/if}
  {:else if section === 'storage'}
    <QuickMetricRow
      label="Disk"
      value={disk ? `${Math.round(disk.percent_used ?? 0)}% used` : 'Reading…'}
      detail={disk
        ? `${formatBytes(disk.available_bytes)} free of ${formatBytes(disk.total_bytes)}`
        : cleanupCapability?.reason ?? null}
      actionLabel="Open storage"
      onclick={() => handleOpenRoute('storage')}
    />
  {:else if section === 'categories'}
    {#if scan}
      <section class="quick-list-section divide-y divide-border" aria-label="Storage categories">
        {#each scan.categories as cat (cat.category)}
          <div class="flex items-center justify-between gap-2 px-3 py-2 text-meta">
            <span class="truncate text-foreground font-medium">{cat.display_name}</span>
            <span class="font-mono tabular-nums text-muted-foreground whitespace-nowrap">{formatBytes(cat.total_bytes)}</span>
          </div>
        {/each}
      </section>
    {:else if scanStore.isScanning}
      <div class="py-4 text-center space-y-2">
        <LoadingSpinner size={16} class="mx-auto text-muted-foreground" />
        <p class="text-meta text-muted-foreground">Scanning caches...</p>
      </div>
    {/if}
  {:else if section === 'agent_activity'}
    <section class="quick-list-section" aria-label="Active AI and services">
      <div class="flex items-center justify-between gap-2 px-1 pb-1">
        <span class="text-meta font-semibold text-foreground">
          AI Activity{activeCount > 0 ? ` · ${activeCount} active` : ''}
        </span>
        <button
          type="button"
          class="inline-flex shrink-0 items-center gap-1 rounded text-caption text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onclick={() => handleOpenRoute('projects')}
          aria-label="Open AI Activity in the main window"
        >
          Open <ArrowRight size={11} aria-hidden="true" />
        </button>
      </div>

      {#if !aiAvailable}
        <p class="px-1 text-caption text-muted-foreground">{aiCapability?.reason ?? 'AI integrations are unavailable on this platform.'}</p>
      {:else if aiRows.length === 0}
        <p class="px-1 text-caption text-muted-foreground">No connected providers or observed sessions.</p>
      {:else}
        <ul class="divide-y divide-border">
          {#each visibleAiRows as row (row.id)}
            <li class="flex min-w-0 items-start gap-2.5 px-1 py-2">
              <BrandIcon identity={row.identity} label={row.name} size={20} class="mt-0.5" />
              <div class="min-w-0 flex-1">
                <div class="flex min-w-0 items-baseline justify-between gap-2">
                  <span class="min-w-0 break-words text-meta font-medium text-foreground">{row.name}</span>
                  {#if row.sessions.length > 0}
                    <span class="shrink-0 text-caption text-success">{row.sessions.length} active</span>
                  {/if}
                </div>
                <p class="text-caption leading-snug text-muted-foreground">
                  {row.provider
                    ? formatQuickProviderUsage(
                        row.provider,
                        usageStore.isProviderLoading(row.provider.id),
                        !!usageStore.snapshot && Date.now() / 1000 - usageStore.snapshot.fetched_at > 300
                      )
                    : `${row.sessions.length} observed session${row.sessions.length === 1 ? '' : 's'}`}
                </p>
              </div>
            </li>
          {/each}
          {#if hiddenAiCount > 0}
            <li class="px-1 pt-1.5 text-caption text-muted-foreground">+{hiddenAiCount} more in AI Activity</li>
          {/if}
        </ul>
      {/if}
    </section>
  {:else if section === 'awake'}
    <section class="quick-list-section flex items-center justify-between gap-2" aria-label="Keep Awake">
      <div class="flex items-center gap-2 min-w-0">
        <Moon size={15} class="text-muted-foreground shrink-0" aria-hidden="true" />
        <div class="min-w-0">
          <p class="text-meta font-medium text-foreground">Keep Awake</p>
          <p class="text-caption text-muted-foreground truncate">
            {#if !awakeAvailable}
              {awakeCapability?.reason ?? 'Unavailable on this platform.'}
            {:else if awakeState.is_active}
              {awakeState.active_process_name ?? (awakeState.trigger_source === 'manual' ? 'Manual session' : 'Rule matched')}{awakeRemaining ? ` · ${awakeRemaining}` : ''}
            {:else}
              Inactive
            {/if}
          </p>
        </div>
      </div>
      {#if awakeAvailable}
        <Button
          variant="secondary"
          size="sm"
          class="text-meta shrink-0"
          disabled={awakeStore.isLoading}
          onclick={() => (awakeState.is_active ? awakeStore.disableManual() : awakeStore.setManual(3600, 'prevent_system_sleep'))}
          ariaLabel={awakeState.is_active ? 'Stop keeping this Mac awake' : 'Keep this Mac awake for one hour'}
        >
          <span>{awakeState.is_active ? 'Stop' : '1 h'}</span>
        </Button>
      {/if}
    </section>
  {/if}
{/snippet}
