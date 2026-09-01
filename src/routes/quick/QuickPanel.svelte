<script lang="ts">
  import { onMount } from 'svelte';
  import type { AiProviderUsage, ControlCenterQuickSummary, AgentQuickSummary } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { memoryStore } from '../../lib/stores/memory.svelte';
  import { awakeStore } from '../../lib/stores/awake.svelte';
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import { platformCapabilitiesStore } from '../../lib/stores/platformCapabilities.svelte';
  import { usageStore } from '../../lib/stores/usage.svelte';
  import { formatBytes, formatTimeAgo, formatTimeUntil, formatResetDate } from '../../lib/utils/format';
  import { isQuickPanelDismissShortcut, projectAiProviders } from '../../lib/utils/quickPanel';
  import {
    isTauri,
    tauriHideCurrentWindow,
    tauriGetAiControlQuickSummary,
    tauriGetAgentQuickSummary,
    tauriOpenDashboard,
    tauriStartWindowDrag,
  } from '../../lib/utils/tauri';
  import { APP_VERSION, formatVersion } from '../../lib/utils/version';
  import Button from '../../lib/components/Button.svelte';
  import ProgressBar from '../../lib/components/ProgressBar.svelte';
  import QuickUsageGauges from '../../lib/components/QuickUsageGauges.svelte';
  import CleanResultModal from '../../lib/components/CleanResultModal.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import {
    Sparkles,
    Trash2,
    HardDrive,
    Activity,
    Bot,
    Moon,
    Flame,
    RotateCw,
    X,
    LayoutDashboard,
    Settings,
    Code2,
    Container,
    Boxes,
    ArrowRight,
  } from 'lucide-svelte';

  let panelActive = false;
  let showResultModal = $state(false);
  let controlSummary = $state<ControlCenterQuickSummary | null>(null);
  let agentSummary = $state<AgentQuickSummary | null>(null);
  let settings = $derived(settingsStore.settings);
  let disk = $derived(memoryStore.disk);
  let memory = $derived(memoryStore.memory);
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
  let cleanupAvailable = $derived(cleanupCapability?.status === 'available');
  let memoryAvailable = $derived(platformCapabilitiesStore.isInspectable('memory_metrics'));
  let awakeAvailable = $derived(awakeCapability?.status === 'available');
  let aiAvailable = $derived(aiCapability?.status === 'available');

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
    if (quickCleanableBytes > 0) {
      return 'ready';
    }
    return 'clean';
  });

  function formatDuration(seconds: number) {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    return hours > 0 ? `${hours}h ${minutes}m` : `${Math.max(minutes, 1)}m`;
  }

  function hasSection(section: typeof settings.quick_panel_sections[number]) {
    return settings.quick_panel_sections.includes(section);
  }

  async function activatePanel() {
    if (panelActive) return;
    panelActive = true;
    await settingsStore.load(true);
    await platformCapabilitiesStore.load(true);
    if (!panelActive) return;
    if (awakeAvailable) void awakeStore.refresh();
    if (hasSection('storage') && cleanupAvailable) void memoryStore.refreshDisk();
    if (hasSection('memory') && memoryAvailable) memoryStore.startPolling(3000);
    if (
      (hasSection('ai_usage') || hasSection('agent_activity')) &&
      aiAvailable &&
      settings.quick_panel_ai_providers.length > 0
    ) {
      void usageStore.refreshIfStale();
    }
    if (hasSection('ai_control') && aiAvailable) {
      // Cached backend projection only: no provider calls, scans, or hidden polling.
      void tauriGetAiControlQuickSummary().then((summary) => {
        if (panelActive) controlSummary = summary;
      });
    }
    if (hasSection('agent_activity') && aiAvailable) {
      void tauriGetAgentQuickSummary().then((summary) => {
        if (panelActive) agentSummary = summary;
      });
    }
    if ((hasSection('cleanup') || hasSection('categories')) && cleanupAvailable) {
      await scanStore.init();
      if (panelActive && scanStore.isStale()) void scanStore.runScan();
    }
  }

  function deactivatePanel() {
    if (!panelActive) return;
    panelActive = false;
    if (hasSection('memory')) memoryStore.stopPolling();
  }

  onMount(() => {
    let disposed = false;
    let unlistenFocus: (() => void) | undefined;

    const closeOnShortcut = (event: KeyboardEvent) => {
      if (isQuickPanelDismissShortcut(event.key, event.metaKey)) {
        event.preventDefault();
        event.stopImmediatePropagation();
        handleClose();
      }
    };
    window.addEventListener('keydown', closeOnShortcut, true);

    if (!isTauri) {
      void activatePanel();
    } else {
      void import('@tauri-apps/api/webviewWindow').then(async ({ getCurrentWebviewWindow }) => {
        const currentWindow = getCurrentWebviewWindow();
        unlistenFocus = await currentWindow.onFocusChanged(({ payload: focused }) => {
          if (focused) {
            void activatePanel();
          } else {
            deactivatePanel();
            void currentWindow.hide();
          }
        });
        if (!disposed && await currentWindow.isVisible()) void activatePanel();
      });
    }

    return () => {
      disposed = true;
      unlistenFocus?.();
      window.removeEventListener('keydown', closeOnShortcut, true);
      deactivatePanel();
    };
  });

  async function handleCleanSafe() {
    scanStore.selectQuickCleanDefaults(settings);
    const result = await scanStore.cleanSelected();
    if (result) {
      showResultModal = true;
    }
  }

  function handleOpenDashboard() {
    deactivatePanel();
    void tauriOpenDashboard();
  }

  function handleClose() {
    deactivatePanel();
    void tauriHideCurrentWindow();
  }

  function providerValue(provider: AiProviderUsage) {
    if (usageStore.isProviderLoading(provider.id)) {
      return '';
    }

    if (provider.windows.length > 0) {
      const window = provider.windows[0];
      const percent = Math.round(window.used_percent ?? 0);
      if (window.resets_at) {
        const timeUntil = formatTimeUntil(window.resets_at);
        if (timeUntil) {
          return `${percent}% used (${timeUntil})`;
        }
      }
      return `${percent}% used`;
    }

    if (provider.summary.local_sessions != null) return `${provider.summary.local_sessions} sessions`;
    if (provider.summary.usage_usd != null) return `$${provider.summary.usage_usd.toFixed(2)}`;
    return provider.connected ? 'Connected' : provider.installed ? 'Available' : 'Not installed';
  }

  function providerTitle(provider: AiProviderUsage) {
    if (usageStore.isProviderLoading(provider.id)) {
      return `${provider.name}: Loading live quota...`;
    }
    if (provider.windows.length > 0) {
      return provider.windows
        .map((w) => {
          const time = w.resets_at ? ` (resets in ${formatTimeUntil(w.resets_at)})` : '';
          return `${w.label}: ${Math.round(w.used_percent ?? 0)}% used${time}`;
        })
        .join(' · ');
    }
    return provider.status_message || provider.auth_label || provider.name;
  }

  function handleWindowDrag(event: MouseEvent) {
    if (event.button !== 0) return;
    const target = event.target;
    if (target instanceof Element && target.closest('.no-drag')) return;
    void tauriStartWindowDrag().catch(() => undefined);
  }
</script>

<div class="w-full h-full min-h-[480px] max-h-[520px] bg-background border border-border/80 rounded-2xl flex flex-col justify-between p-4 select-none shadow-2xl text-foreground font-sans overflow-hidden relative">
  <!-- Header — draggable, buttons are no-drag -->
  <div
    class="flex shrink-0 items-center justify-between pb-3 border-b border-border/60 relative titlebar-drag-region"
    role="presentation"
    onmousedown={handleWindowDrag}
  >
    <div class="flex items-center space-x-2">
      <svg class="h-6 w-6 rounded-lg shrink-0 shadow-sm" viewBox="0 0 1024 1024">
        <defs>
          <linearGradient id="quick-bg-grad" x1="160" y1="112" x2="864" y2="912" gradientUnits="userSpaceOnUse">
            <stop stop-color="#27272f"/>
            <stop offset="1" stop-color="#101014"/>
          </linearGradient>
        </defs>
        <rect width="1024" height="1024" rx="220" fill="url(#quick-bg-grad)"/>
        <path d="M292 300h466v116L486 650h282v116H266V650l270-234H292z" fill="#fff"/>
        <circle cx="758" cy="300" r="44" fill="#34d399"/>
      </svg>
      <span class="text-sm font-semibold tracking-tight">Zenith</span>
    </div>
    <div class="flex items-center space-x-1 no-drag">
      {#if awakeState.is_active}
        <div class="flex items-center gap-1.5 px-2 py-0.5 rounded-full bg-warning/15 text-warning text-caption font-medium border border-warning/20">
          <span class="h-1.5 w-1.5 rounded-full bg-warning animate-pulse-soft"></span>
          <span>Awake</span>
        </div>
      {/if}
      <Button variant="ghost" size="icon" class="h-7 w-7 text-muted-foreground hover:text-foreground" onclick={handleOpenDashboard} ariaLabel="Open settings"><Settings size={14} /></Button>
      <Button variant="ghost" size="icon" class="h-7 w-7 text-muted-foreground hover:text-foreground hover:bg-secondary" onclick={handleClose} ariaLabel="Close quick panel" title="Close"><X size={15} /></Button>
    </div>
  </div>

  <!-- Body Content -->
  <div class="min-h-0 flex-1 overflow-y-auto py-3 space-y-3 pr-1">
    {#each settings.quick_panel_sections as section}
      {#if section === 'cleanup'}
        <!-- Action Hero Card -->
        <div class="p-3.5 bg-secondary/60 border border-border/70 rounded-xl flex items-center justify-between shadow-xs">
          <div>
            <div class="text-caption font-semibold text-muted-foreground uppercase tracking-wider">
              {cleanupState === 'ready' || cleanupState === 'refreshing' ? 'Ready to Clean' : 'System Status'}
            </div>
            <div class="text-2xl font-bold font-mono text-foreground mt-0.5">
              {#if cleanupState === 'unknown'}
                <span>—</span>
              {:else if cleanupState === 'scanning'}
                <span class="text-base text-muted-foreground font-sans font-medium">Scanning…</span>
              {:else}
                <span>{formatBytes(quickCleanableBytes)}</span>
              {/if}
            </div>
            <div class="text-caption font-medium mt-0.5 flex items-center gap-1">
              {#if cleanupState === 'unknown'}
                <span class="text-muted-foreground">Run a scan to check safe caches</span>
              {:else if cleanupState === 'scanning'}
                <span class="text-muted-foreground">Checking development caches</span>
              {:else if cleanupState === 'refreshing'}
                <span class="text-warning">Refreshing scan…</span>
              {:else if cleanupState === 'ready'}
                <span class="text-success">Safe cleanup available</span>
              {:else}
                <span class="text-success">Development caches clean</span>
              {/if}
            </div>
          </div>
          <Button
            variant="primary"
            size="sm"
            disabled={!cleanupAvailable || scanStore.isScanning || scanStore.isCleaning || !scan || quickCleanableBytes === 0}
            onclick={handleCleanSafe}
            class="gap-1.5 min-w-[88px] text-xs font-semibold py-2 px-3"
          >
            {#if scanStore.isCleaning}
              <DeletingDots size="xs" />
              <span>Cleaning</span>
            {:else}
              <Trash2 size={13} />
              <span>Clean Safe</span>
            {/if}
          </Button>
        </div>
        {#if !cleanupAvailable}
          <p class="mt-2 text-caption text-warning">
            {cleanupCapability?.reason ?? 'Cleanup is unavailable on this platform.'}
          </p>
        {/if}
      {:else if section === 'storage' && disk}
        <!-- Storage Gauge -->
        <div class="space-y-1.5 p-2.5 rounded-xl border border-border/50 bg-card/40">
          <div class="flex justify-between text-xs font-medium">
            <span class="text-muted-foreground">Mac Storage</span>
            <span class="font-mono text-foreground">{formatBytes(disk.used_bytes)} / {formatBytes(disk.total_bytes)}</span>
          </div>
          <ProgressBar value={disk.percent_used ?? 0} height="h-2" />
        </div>
      {:else if section === 'memory' && memory && memoryAvailable}
        <!-- Memory Gauge -->
        <div class="space-y-1.5 p-2.5 rounded-xl border border-border/50 bg-card/40">
          <div class="flex justify-between text-xs font-medium">
            <div class="flex items-center gap-1 text-muted-foreground">
              <span>Memory</span>
              <span class="text-caption font-mono px-1 rounded bg-secondary text-foreground capitalize">
                {memory.pressure}
              </span>
            </div>
            <span class="font-mono text-foreground">{formatBytes(memory.used_bytes)} / {formatBytes(memory.total_bytes)}</span>
          </div>
          <ProgressBar
            value={(memory.used_bytes / memory.total_bytes) * 100}
            height="h-2"
            color={memory.pressure === 'critical' ? 'bg-destructive' : memory.pressure === 'warning' ? 'bg-warning' : 'bg-success'}
          />
        </div>
      {:else if section === 'ai_usage'}
        <!-- AI Usage Quick List -->
        <div class="space-y-1.5 rounded-xl border border-border/60 bg-card/40 p-2.5">
          <div class="flex items-center justify-between px-1 pb-1">
            <div class="flex items-center gap-1.5 text-meta font-semibold uppercase tracking-wider text-muted-foreground">
              <Sparkles size={12} class="text-violet-400" /> AI Usage
            </div>
            <button
              type="button"
              class="text-muted-foreground hover:text-foreground p-0.5"
              disabled={!aiAvailable || usageStore.isLoading}
              onclick={() => {
                if (aiAvailable) void usageStore.refresh(true);
              }}
              aria-label="Refresh AI usage"
            >
              <RotateCw size={12} class={usageStore.isLoading ? 'animate-gentle-spin' : ''} />
            </button>
          </div>
          {#if settings.quick_panel_ai_providers.length === 0}
            <div class="py-2 text-center text-caption text-muted-foreground">Configure in Settings.</div>
          {:else if usageStore.isLoading && !usageStore.snapshot}
            <div class="py-2 text-center text-caption text-muted-foreground">Reading accounts…</div>
          {:else if selectedProviders.length}
            {#each selectedProviders as provider}
              <div
                class="flex items-center justify-between gap-2 rounded-lg px-1.5 py-1.5 text-xs hover:bg-secondary/40 transition-colors"
                title={providerTitle(provider)}
              >
                <div class="flex min-w-0 items-center gap-2">
                  <Bot size={13} class={provider.connected ? 'text-success' : 'text-muted-foreground'} />
                  <span class="truncate font-medium">{provider.name}</span>
                </div>
                {#if usageStore.isProviderLoading(provider.id)}
                  <span class="shrink-0 inline-flex items-center text-muted-foreground/70" title="Loading live quota...">
                    <RotateCw size={11} class="animate-spin" />
                  </span>
                {:else}
                  <QuickUsageGauges windows={provider.windows} fallback={providerValue(provider)} />
                {/if}
              </div>
            {/each}
          {:else}
            <div class="py-2 text-center text-caption text-muted-foreground">Configure in Settings.</div>
          {/if}
        </div>
      {:else if section === 'categories'}
        {#if scan}
          <div class="space-y-1.5">
            {#each scan.categories as cat}
              <div class="flex items-center justify-between py-1.5 px-2 rounded-lg hover:bg-secondary/40 text-xs">
                <div class="flex items-center gap-2">
                  {#if cat.category === 'ai'}<Sparkles size={14} class="text-purple-400" />
                  {:else if cat.category === 'developer'}<Code2 size={14} class="text-blue-400" />
                  {:else if cat.category === 'container'}<Container size={14} class="text-cyan-400" />
                  {:else}<Boxes size={14} class="text-warning" />{/if}
                  <span class="text-foreground font-medium">{cat.display_name}</span>
                </div>
                <span class="font-mono text-muted-foreground">{formatBytes(cat.total_bytes)}</span>
              </div>
            {/each}
          </div>
        {:else if scanStore.isScanning}
          <div class="py-4 text-center space-y-2">
            <RotateCw size={16} class="animate-gentle-spin mx-auto text-muted-foreground" />
            <p class="text-xs text-muted-foreground">Scanning caches...</p>
          </div>
        {/if}
      {:else if section === 'ai_control'}
        <div class="space-y-2 rounded-xl border border-border/60 bg-card/40 p-2.5">
          <div class="flex items-center justify-between px-1">
            <div class="flex items-center gap-1.5 text-meta font-semibold uppercase tracking-wider text-muted-foreground"><Sparkles size={12} class="text-violet-400" />AI Control</div>
            <span class="text-micro capitalize text-muted-foreground">{controlSummary?.quality ?? 'not cached'}</span>
          </div>
          {#if controlSummary}
            <div class="grid grid-cols-3 gap-1.5 text-center">
              <div class="rounded-lg bg-secondary/50 p-2"><p class="font-mono text-sm font-semibold">{controlSummary.active_sessions}</p><p class="text-micro text-muted-foreground">Sessions</p></div>
              <div class="rounded-lg bg-secondary/50 p-2"><p class="font-mono text-sm font-semibold">{controlSummary.budget_alerts}</p><p class="text-micro text-muted-foreground">Alerts</p></div>
              <div class="rounded-lg bg-secondary/50 p-2"><p class="font-mono text-sm font-semibold">{controlSummary.safety_findings}</p><p class="text-micro text-muted-foreground">Safety</p></div>
            </div>
          {:else}<p class="px-1 py-2 text-caption text-muted-foreground">Open AI Control in the dashboard to create a cached snapshot.</p>{/if}
        </div>
      {:else if section === 'agent_activity'}
        <div class="space-y-2 rounded-xl border border-border/60 bg-card/40 p-2.5">
          <div class="flex items-center justify-between px-1">
            <div class="flex items-center gap-1.5 text-meta font-semibold uppercase tracking-wider text-muted-foreground">
              <Bot size={12} class="text-primary" /> AI & Agents
            </div>
            <button
              type="button"
              class="text-caption text-muted-foreground hover:text-foreground inline-flex items-center gap-1"
              onclick={handleOpenDashboard}
            >
              Open AI Activity <ArrowRight size={10} />
            </button>
          </div>
          {#if selectedProviders.length > 0}
            <div class="space-y-0.5 rounded-lg border border-border/40 bg-background/30 p-1">
              {#each selectedProviders as provider (provider.id)}
                <div
                  class="flex items-center justify-between gap-2 rounded-md px-1.5 py-1 text-xs hover:bg-secondary/40 transition-colors"
                  title={providerTitle(provider)}
                >
                  <div class="flex min-w-0 items-center gap-1.5">
                    <span class="h-1.5 w-1.5 shrink-0 rounded-full {usageStore.isProviderLoading(provider.id) ? 'bg-muted-foreground/40 animate-pulse' : provider.connected ? 'bg-success' : 'bg-muted-foreground/50'}"></span>
                    <span class="truncate text-muted-foreground">{provider.name}</span>
                  </div>
                  {#if usageStore.isProviderLoading(provider.id)}
                    <span class="shrink-0 inline-flex items-center text-muted-foreground/70" title="Loading live quota...">
                      <RotateCw size={11} class="animate-spin" />
                    </span>
                  {:else}
                    <QuickUsageGauges windows={provider.windows} fallback={providerValue(provider)} />
                  {/if}
                </div>
              {/each}
            </div>
          {/if}

          {#if agentSummary && agentSummary.active_count > 0}
            <div class="grid grid-cols-2 gap-1.5 text-center">
              <div class="rounded-lg bg-secondary/50 p-2">
                <p class="font-mono text-sm font-semibold">{agentSummary.active_count}</p>
                <p class="text-micro text-muted-foreground">Active</p>
              </div>
              <div class="rounded-lg bg-secondary/50 p-2">
                <p class="font-mono text-sm font-semibold {agentSummary.attention_count > 0 ? 'text-destructive' : ''}">{agentSummary.attention_count}</p>
                <p class="text-micro text-muted-foreground">Attention</p>
              </div>
            </div>
            {#if agentSummary.sessions.length > 0}
              <div class="divide-y divide-border/40 rounded-lg border border-border/50 bg-background/30 overflow-hidden">
                {#each agentSummary.sessions as session}
                  <div class="flex items-center justify-between px-2 py-1.5 text-xs">
                    <div class="flex items-center gap-1.5 min-w-0">
                      <span class="font-medium truncate">{session.tool_name}</span>
                      <span class="text-caption text-muted-foreground truncate">in {session.project_name}</span>
                    </div>
                    <span class="font-mono text-caption text-muted-foreground shrink-0 ml-2">
                      {formatDuration(session.elapsed_seconds)}
                    </span>
                  </div>
                {/each}
              </div>
            {/if}
          {:else if selectedProviders.length === 0}
            <p class="px-1 py-2 text-caption text-muted-foreground">No active agent sessions detected.</p>
          {/if}
        </div>
      {/if}
    {/each}
  </div>

  <!-- Footer -->
  <div class="shrink-0 pt-3 border-t border-border/60 flex items-center justify-between gap-2">
    <div class="flex items-center gap-1.5 text-meta text-muted-foreground">
      <span>Last scan {formatTimeAgo(scan?.finished_at)}</span>
      <button
        type="button"
        disabled={!cleanupAvailable || scanStore.isScanning || scanStore.isCleaning}
        onclick={() => {
          if (cleanupAvailable) void scanStore.runScan();
        }}
        class="p-1 rounded-md hover:bg-secondary text-muted-foreground hover:text-foreground transition-colors disabled:opacity-50"
        title={cleanupAvailable ? 'Rescan storage' : (cleanupCapability?.reason ?? 'Storage cleanup is unavailable on this platform.')}
      >
        <RotateCw size={11} class={scanStore.isScanning ? 'animate-gentle-spin' : ''} />
      </button>
    </div>
    <div class="flex items-center gap-2">
      <span class="text-caption font-mono text-muted-foreground/60 select-none">{formatVersion(APP_VERSION)}</span>
      <Button
        variant="secondary"
        size="sm"
        onclick={handleOpenDashboard}
        class="gap-1.5 text-xs font-medium"
      >
        <span>Open Zenith</span>
        <ArrowRight size={13} />
      </Button>
    </div>
  </div>

  {#if showResultModal && scanStore.lastCleanResult}
    <CleanResultModal result={scanStore.lastCleanResult} onClose={() => (showResultModal = false)} />
  {/if}
</div>
