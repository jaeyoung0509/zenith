<script lang="ts">
  import { onMount } from 'svelte';
  import type {
    DashboardRoute,
    AgentQuickSummary,
    QuickPanelSection,
    ScanItem,
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
  import { cleanupSummaryState } from '../../lib/utils/cleanupSummary';
  import { batteryChargeStateLabel, memoryPressureLabel } from '../../lib/utils/systemReadings';
  import {
    handleQuickPanelFocusChanged,
    isAcceleratorPressed,
    isQuickPanelDismissShortcut,
    platformAccelerator,
    quickPanelHeight,
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
  import QuickSafeReviewDialog from '../../lib/components/QuickSafeReviewDialog.svelte';
  import QuickCleanupDetailsDialog from '../../lib/components/QuickCleanupDetailsDialog.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import InlineNotice from '../../lib/components/InlineNotice.svelte';
  import PreviewModeIndicator from '../../lib/components/PreviewModeIndicator.svelte';
  import {
    ArrowRight,
    HardDrive,
    Moon,
    RefreshCw,
    Settings,
    X,
  } from '@lucide/svelte';

  /** Keep the panel useful without turning it into an agent inventory. */
  const AI_ROW_LIMIT = 5;

  let panelActive = false;
  let showResultModal = $state(false);
  let showCleanupDetails = $state(false);
  let safeReview = $state<{ scanId: string; items: ScanItem[]; partial: boolean } | null>(null);
  let usageNow = $state(Date.now());
  let usageClock: ReturnType<typeof setInterval> | undefined;
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
  let quickSafeItems = $derived(
    scan?.categories.flatMap((category) => category.items.filter((item) =>
      scanStore.isQuickCleanEligible(category.category, item, settings)
    )) ?? []
  );

  let cleanupState = $derived(cleanupSummaryState({
    available: cleanupAvailable,
    hasScan: !!scan,
    scanning: scanStore.isScanning,
    cleaning: scanStore.isCleaning,
    freshness: scanStore.freshness,
    cleanableBytes: quickCleanableBytes,
  }));

  let cleanupValue = $derived(formatBytes(quickCleanableBytes));

  let cleanupBusy = $derived(
    cleanupState === 'scanning' || cleanupState === 'refreshing' || cleanupState === 'cleaning'
  );

  let cleanupActionLabel = $derived(
    cleanupState === 'cleaning'
      ? 'View cleanup'
      : cleanupBusy
        ? 'View scan'
        : cleanupState === 'partial' && quickSafeItems.length > 0
          ? 'Review Safe'
          : 'Open Storage'
  );

  let cleanupDetail = $derived.by(() => {
    switch (cleanupState) {
      case 'unknown':
        return 'Run a scan to check safe caches.';
      case 'unavailable':
        return cleanupCapability?.reason ?? 'Storage cleanup is unavailable here.';
      case 'scanning':
      case 'refreshing':
      case 'cleaning':
        return '';
      case 'stale':
        return 'Scan again to verify safe cleanup.';
      case 'failed':
        return 'Scan could not finish. Open Storage for details.';
      case 'partial':
        return 'Some folders weren’t checked. Review measured items.';
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
  let panelShell: HTMLDivElement;
  let panelHeader: HTMLDivElement;
  let panelFooter: HTMLDivElement;
  let panelContent: HTMLDivElement;
  let resizePanelToContent: (() => void) | undefined;

  async function activatePanel() {
    if (panelActive) return;
    panelActive = true;
    usageNow = Date.now();
    usageClock = setInterval(() => (usageNow = Date.now()), 30_000);
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
    resizePanelToContent?.();
  }

  function deactivatePanel() {
    if (!panelActive) return;
    panelActive = false;
    if (usageClock !== undefined) clearInterval(usageClock);
    usageClock = undefined;
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
    let cleanupResize: (() => void) | undefined;

    const closeOnShortcut = (event: KeyboardEvent) => {
      // Let the active native dialog consume Escape before dismissing its window.
      if (event.key === 'Escape' && document.querySelector('dialog[open]')) return;
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
      void Promise.all([
        import('@tauri-apps/api/webviewWindow'),
        import('@tauri-apps/api/dpi'),
        import('@tauri-apps/api/window'),
      ]).then(async ([{ getCurrentWebviewWindow }, { LogicalSize }, { currentMonitor }]) => {
        if (disposed) return;
        const currentWindow = getCurrentWebviewWindow();
        let resizeTimer: number | undefined;
        const resizeToContent = () => {
          if (resizeTimer !== undefined) window.clearTimeout(resizeTimer);
          resizeTimer = window.setTimeout(() => {
            if (!panelActive || !panelShell || !panelHeader || !panelFooter || !panelContent) return;
            void (async () => {
              let maximumHeight = 740;
              try {
                const monitor = await currentMonitor();
                if (monitor) {
                  maximumHeight = Math.min(
                    maximumHeight,
                    Math.floor(monitor.workArea.size.height / monitor.scaleFactor - 24)
                  );
                }
              } catch {
                // Keep the configured maximum when the active monitor is unavailable.
              }
              const chromeHeight = panelHeader.offsetHeight + panelFooter.offsetHeight + 20;
              const nextHeight = quickPanelHeight(
                panelContent.scrollHeight,
                chromeHeight,
                maximumHeight
              );
              if (Math.abs(panelShell.clientHeight - nextHeight) >= 16) {
                await currentWindow
                  .setSize(new LogicalSize(panelShell.clientWidth, nextHeight))
                  .catch(() => undefined);
              }
            })();
          }, 180);
        };
        const observer = new ResizeObserver(resizeToContent);
        if (panelContent) observer.observe(panelContent);
        resizePanelToContent = resizeToContent;
        cleanupResize = () => {
          observer.disconnect();
          if (resizeTimer !== undefined) window.clearTimeout(resizeTimer);
          if (resizePanelToContent === resizeToContent) resizePanelToContent = undefined;
        };

        const unlisten = await currentWindow.onFocusChanged(({ payload: focused }) => {
          handleQuickPanelFocusChanged(focused, {
            activate: activatePanel,
            deactivate: deactivatePanel,
          });
        });
        if (disposed) {
          unlisten();
          cleanupResize?.();
          return;
        }
        unlistenFocus = unlisten;
        if (!disposed && await currentWindow.isVisible()) void activatePanel();
      });
    }

    return () => {
      disposed = true;
      unlistenFocus?.();
      cleanupResize?.();
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

  function openSafeReview() {
    if (!scan || !scanStore.canClean || quickSafeItems.length === 0) return;
    safeReview = {
      scanId: scan.scan_id,
      items: quickSafeItems,
      partial: scan.quality === 'partial',
    };
  }

  function confirmSafeReview(selectedItemIds: string[]) {
    if (!safeReview || scan?.scan_id !== safeReview.scanId || !scanStore.canClean) return;
    const scanId = safeReview.scanId;
    safeReview = null;
    queueMicrotask(() => document.getElementById('quick-cleanup-summary')?.focus());
    void scanStore.reviewedQuickCleanSafe(scanId, selectedItemIds).then((result) => {
      if (result) showResultModal = true;
    });
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

<div bind:this={panelShell} data-material={isTauri() && platformContextStore.context?.platform === 'macos' ? 'native' : 'web'} class="quick-liquid-shell w-full h-full rounded-2xl flex flex-col select-none text-foreground font-sans overflow-hidden relative">
  <!-- Header — draggable, buttons are no-drag -->
  <div
    bind:this={panelHeader}
    class="quick-liquid-chrome flex shrink-0 items-center justify-between gap-2 px-4 py-2.5 border-b border-border relative titlebar-drag-region"
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
    <div bind:this={panelContent} class="quick-panel-content">
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
  </div>

  <!-- Footer -->
  <div bind:this={panelFooter} class="quick-liquid-chrome shrink-0 pt-2 border-t px-4 pb-2 flex items-center justify-between gap-2">
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
          <RefreshCw size={12} aria-hidden="true" />
        </span>
      </button>
    </div>
    <div class="flex items-center gap-1.5 shrink-0">
      <PreviewModeIndicator />
      <span class="quick-version text-caption font-mono text-muted-foreground select-none">{formatVersion(APP_VERSION)}</span>
      <Button
        variant="secondary"
        size="sm"
        id="quick-open-zenith-button"
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
      returnFocusTargetId="quick-open-zenith-button"
    />
  {/if}
  {#if safeReview}
    <QuickSafeReviewDialog
      items={safeReview.items}
      partial={safeReview.partial}
      disabled={scan?.scan_id !== safeReview.scanId || !scanStore.canClean}
      onCancel={() => (safeReview = null)}
      onConfirm={confirmSafeReview}
    />
  {/if}
  {#if showCleanupDetails && scan}
    <QuickCleanupDetailsDialog
      {scan}
      quickEligibleCount={quickSafeItems.length}
      busy={cleanupBusy}
      onClose={() => (showCleanupDetails = false)}
      onReview={() => { showCleanupDetails = false; handleOpenRoute('storage'); }}
      onRescan={() => { showCleanupDetails = false; void scanStore.runScan(); }}
    />
  {/if}
</div>

{#snippet sectionCell(section: QuickPanelSection)}
  {#if section === 'cleanup'}
    <section id="quick-cleanup-summary" tabindex="-1" class="quick-list-section quick-cleanup-summary outline-none" aria-label="Cleanup">
      <div class="flex items-center justify-between gap-3">
        <div class="flex min-w-0 items-center gap-2.5">
          <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-accent/60 text-primary" aria-hidden="true"><HardDrive size={16} strokeWidth={1.75} /></span>
          <div class="min-w-0" aria-busy={cleanupBusy} title={cleanupDetail}>
            <p class="text-meta font-semibold text-foreground">Cleanup</p>
            {#if cleanupBusy}
              <p class="quick-cleanup-status mt-1 flex items-center gap-2 text-caption text-muted-foreground" role="status">
                <DeletingDots size="sm" class="shrink-0 text-primary" />
                <span>{cleanupState === 'cleaning' ? 'Cleaning safe caches' : scanStore.isRefreshingAfterClean ? 'Checking the result' : 'Checking storage'}</span>
              </p>
            {:else if cleanupState === 'ready' || (cleanupState === 'partial' && quickCleanableBytes > 0)}
              <p class="mt-0.5 text-caption text-muted-foreground"><span class="font-semibold tabular-nums text-foreground">{cleanupValue}</span> to review{cleanupState === 'partial' ? ' · Partial scan' : ''}</p>
            {:else}
              <p class="mt-0.5 text-caption text-muted-foreground">{cleanupState === 'clean' ? 'No safe caches found' : cleanupState === 'failed' ? 'Scan failed' : cleanupState === 'unavailable' ? 'Unavailable' : cleanupState === 'partial' ? 'Partial scan · No Quick Clean items' : 'Scan needed'}</p>
            {/if}
          </div>
        </div>
        {#if cleanupAvailable && (cleanupState === 'stale' || cleanupState === 'unknown' || cleanupState === 'failed')}
          <Button variant="secondary" size="sm" onclick={() => void scanStore.runScan()} title={cleanupDetail} class="shrink-0">
            {cleanupState === 'unknown' ? 'Scan Now' : 'Scan Again'}
          </Button>
        {:else if cleanupState === 'ready'}
          <Button variant="primary" size="sm" disabled={!scanStore.canClean} onclick={handleCleanSafe} title={cleanupDetail} class="shrink-0">Clean Safe</Button>
        {:else if cleanupState === 'partial' && quickSafeItems.length > 0}
          <Button variant="primary" size="sm" disabled={!scanStore.canClean} onclick={openSafeReview} title="Review measured Safe items in this panel" class="shrink-0">Review Safe</Button>
        {:else if cleanupState === 'partial'}
          <Button variant="secondary" size="sm" onclick={() => (showCleanupDetails = true)} title="See why Quick Clean is unavailable and review items in Storage" class="shrink-0">Details</Button>
        {:else}
          <Button variant="ghost" size="sm" class="gap-1 shrink-0 text-meta text-primary" disabled={!cleanupAvailable} onclick={() => handleOpenRoute('storage')} ariaLabel={`${cleanupActionLabel} in the main window`} title={cleanupDetail}>
            <span>{cleanupActionLabel}</span>
            <ArrowRight size={12} aria-hidden="true" />
          </Button>
        {/if}
      </div>
      {#if scanStore.error && !cleanupBusy}
        <p class="mt-2 text-caption leading-snug text-destructive" role="alert">{scanStore.error}</p>
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
      value={memory ? formatBytes(memory.used_bytes) : memoryAvailable ? 'Reading…' : 'Unavailable'}
      tone={memory?.pressure === 'critical' ? 'critical' : memory?.pressure === 'warning' ? 'warning' : 'default'}
      detail={memory
        ? `of ${formatBytes(memory.total_bytes)} · ${memoryPressureLabel(memory.pressure)} pressure`
        : platformCapabilitiesStore.feature('memory_metrics')?.reason ?? null}
      actionLabel="Open memory detail"
      onclick={() => handleOpenRoute('memory')}
    />
  {:else if section === 'battery'}
    {@const batteryPresent = battery?.presence === 'present'}
    {#if !battery || batteryPresent || battery.presence === 'unavailable'}
      <QuickMetricRow
        label="Battery"
        batteryState={battery?.charge_state}
        batteryPercent={batteryPresent ? battery?.percent ?? null : null}
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
      meter={disk?.percent_used}
      detail={disk
        ? `${formatBytes(disk.available_bytes)} free of ${formatBytes(disk.total_bytes)}`
        : cleanupCapability?.reason ?? null}
      actionLabel="Open storage"
      onclick={() => handleOpenRoute('storage')}
    />
  {:else if section === 'categories'}
    {#if scan && !scanStore.isScanning && !scanStore.isCleaning && !scanStore.isRefreshingAfterClean}
      <section class="quick-list-section divide-y divide-border" aria-label="Storage categories">
        {#each scan.categories as cat (cat.category)}
          <div class="flex items-center justify-between gap-2 px-3 py-2 text-meta">
            <span class="truncate text-foreground font-medium">{cat.display_name}</span>
            <span class="font-mono tabular-nums text-muted-foreground whitespace-nowrap">{formatBytes(cat.total_bytes)}</span>
          </div>
        {/each}
      </section>
    {:else if scanStore.isScanning || scanStore.isCleaning || scanStore.isRefreshingAfterClean}
      <div class="px-1 py-2 text-caption text-muted-foreground">
        Categories will appear when the scan finishes.
      </div>
    {/if}
  {:else if section === 'agent_activity'}
    <section class="quick-list-section" aria-label="Active AI and services">
      <div class="flex items-center justify-between gap-2 px-1 pb-1">
        <span class="inline-flex items-center gap-1.5 text-meta font-semibold text-foreground">
          <span class="h-3.5 w-0.5 rounded-full bg-primary" aria-hidden="true"></span>
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
            <li class="min-w-0 px-1 py-1">
              <div class="min-w-0">
                <div class="flex min-w-0 items-baseline justify-between gap-2">
                  <span class="min-w-0 break-words text-meta font-medium text-foreground">{row.name}</span>
                  {#if row.sessions.length > 0}
                    <span class="shrink-0 text-caption text-primary">{row.sessions.length} active</span>
                  {/if}
                </div>
                <p class="text-caption leading-snug text-muted-foreground">
                  {#if row.provider}
                    {@const loading = usageStore.isProviderLoading(row.provider.id)}
                    {@const stale = !!usageStore.snapshot && usageNow / 1000 - usageStore.snapshot.fetched_at > 300}
                    <span class="inline-flex items-center gap-1.5">
                      {#if loading}<DeletingDots size="xs" class="text-primary" />{/if}
                      <span>{formatQuickProviderUsage(row.provider, loading, stale)}</span>
                      {#if stale && !loading}
                        <button
                          type="button"
                          disabled={usageStore.isLoading}
                          onclick={() => void usageStore.refresh(true)}
                          class="rounded p-0.5 text-primary hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
                          aria-label="Refresh AI usage"
                          title="Refresh AI usage"
                        ><RefreshCw size={12} aria-hidden="true" /></button>
                      {/if}
                    </span>
                  {:else}
                    {row.sessions.length} observed session{row.sessions.length === 1 ? '' : 's'}
                  {/if}
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
