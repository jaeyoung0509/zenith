<script lang="ts">
  import { onMount } from 'svelte';
  import type { AiProviderUsage } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { memoryStore } from '../../lib/stores/memory.svelte';
  import { awakeStore } from '../../lib/stores/awake.svelte';
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import { usageStore } from '../../lib/stores/usage.svelte';
  import { formatBytes, formatTimeAgo } from '../../lib/utils/format';
  import { isQuickPanelDismissShortcut } from '../../lib/utils/quickPanel';
  import { isTauri, tauriHideCurrentWindow, tauriOpenDashboard } from '../../lib/utils/tauri';
  import Button from '../../lib/components/Button.svelte';
  import ProgressBar from '../../lib/components/ProgressBar.svelte';
  import CleanResultModal from '../../lib/components/CleanResultModal.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import {
    RotateCw, Trash2, ArrowRight, Sparkles, Code2, Container, Boxes,
    Eye, Moon, Settings, X, Bot,
  } from 'lucide-svelte';

  let panelActive = false;
  let showResultModal = $state(false);
  let settings = $derived(settingsStore.settings);
  let disk = $derived(memoryStore.disk);
  let memory = $derived(memoryStore.memory);
  let scan = $derived(scanStore.lastScan);
  let awakeState = $derived(awakeStore.state);
  let selectedProviders = $derived.by(() => settings.quick_panel_ai_providers
    .map((id) => usageStore.snapshot?.providers.find((provider) => provider.id === id))
    .filter((provider): provider is AiProviderUsage => Boolean(provider)));

  function hasSection(section: typeof settings.quick_panel_sections[number]) {
    return settings.quick_panel_sections.includes(section);
  }

  async function activatePanel() {
    if (panelActive) return;
    panelActive = true;
    await settingsStore.load(true);
    if (!panelActive) return;
    void awakeStore.refresh();
    if (hasSection('storage')) void memoryStore.refreshDisk();
    if (hasSection('memory')) memoryStore.startPolling(3000);
    if (hasSection('ai_usage') && settings.quick_panel_ai_providers.length) {
      void usageStore.refreshIfStale();
    }
    if (hasSection('cleanup') || hasSection('categories')) {
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

  function handleCleanSafe() {
    scanStore.selectQuickCleanDefaults(settings);
    scanStore.cleanSelected().then((result) => {
      if (result) showResultModal = true;
    });
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
    if (provider.windows[0]) return `${Math.round(provider.windows[0].used_percent)}% used`;
    if (provider.summary.local_sessions != null) return `${provider.summary.local_sessions} sessions`;
    if (provider.summary.usage_usd != null) return `$${provider.summary.usage_usd.toFixed(2)}`;
    return provider.connected ? 'Connected' : provider.installed ? 'Available' : 'Not installed';
  }
</script>

<div class="w-full h-full min-h-[480px] max-h-[520px] bg-background/95 backdrop-blur-xl border border-border/80 rounded-2xl flex flex-col justify-between p-4 select-none shadow-2xl text-foreground font-sans overflow-hidden">
  <!-- Header -->
  <div class="flex items-center justify-between pb-3 border-b border-border/60">
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
    <div class="flex items-center space-x-1">
      {#if awakeState.is_active}
        <div class="flex items-center gap-1 px-2 py-0.5 rounded-full bg-amber-500/15 text-amber-500 text-[10px] font-medium border border-amber-500/20"><Moon size={10} /><span>Awake</span></div>
      {/if}
      <Button variant="ghost" size="icon" class="h-7 w-7 text-muted-foreground hover:text-foreground" onclick={handleOpenDashboard} ariaLabel="Open settings"><Settings size={14} /></Button>
      <Button variant="ghost" size="icon" class="h-7 w-7 text-muted-foreground hover:text-foreground hover:bg-secondary" onclick={handleClose} ariaLabel="Close quick panel" title="Close"><X size={15} /></Button>
    </div>
  </div>

  <!-- Body Content -->
  <div class="flex-1 overflow-y-auto py-3 space-y-3 pr-0.5">
    {#each settings.quick_panel_sections as section}
      {#if section === 'cleanup'}
        <!-- Action Hero Card -->
        <div class="p-3.5 bg-secondary/60 border border-border/70 rounded-xl flex items-center justify-between shadow-xs">
          <div>
            <div class="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">
              {scanStore.safeSelectedBytes > 0 ? 'Ready to Clean' : 'System Status'}
            </div>
            <div class="text-2xl font-bold font-mono text-foreground mt-0.5">
              {formatBytes(scanStore.safeSelectedBytes)}
            </div>
            <div class="text-[10px] text-emerald-500 font-medium mt-0.5 flex items-center gap-1">
              {#if scanStore.safeSelectedBytes > 0}
                <span>Safe cleanup available</span>
              {:else}
                <span>Development caches clean</span>
              {/if}
            </div>
          </div>
          <Button
            variant="primary"
            size="sm"
            disabled={scanStore.isScanning || scanStore.isCleaning || scanStore.safeSelectedBytes === 0}
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
      {:else if section === 'storage' && disk}
        <!-- Storage Gauge -->
        <div class="space-y-1.5 p-2.5 rounded-xl border border-border/50 bg-card/40">
          <div class="flex justify-between text-xs font-medium">
            <span class="text-muted-foreground">Mac Storage</span>
            <span class="font-mono text-foreground">{formatBytes(disk.used_bytes)} / {formatBytes(disk.total_bytes)}</span>
          </div>
          <ProgressBar value={disk.percent_used} height="h-2" />
        </div>
      {:else if section === 'memory' && memory}
        <!-- Memory Gauge -->
        <div class="space-y-1.5 p-2.5 rounded-xl border border-border/50 bg-card/40">
          <div class="flex justify-between text-xs font-medium">
            <div class="flex items-center gap-1 text-muted-foreground">
              <span>Memory</span>
              <span class="text-[10px] font-mono px-1 rounded bg-secondary text-foreground capitalize">
                {memory.pressure}
              </span>
            </div>
            <span class="font-mono text-foreground">{formatBytes(memory.used_bytes)} / {formatBytes(memory.total_bytes)}</span>
          </div>
          <ProgressBar
            value={(memory.used_bytes / memory.total_bytes) * 100}
            height="h-2"
            color={memory.pressure === 'critical' ? 'bg-rose-500' : memory.pressure === 'warning' ? 'bg-amber-500' : 'bg-emerald-500'}
          />
        </div>
      {:else if section === 'ai_usage'}
        <!-- AI Usage Quick List -->
        <div class="space-y-1.5 rounded-xl border border-border/60 bg-card/40 p-2.5">
          <div class="flex items-center justify-between px-1 pb-1">
            <div class="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
              <Sparkles size={12} class="text-violet-400" /> AI Usage
            </div>
            <button
              type="button"
              class="text-muted-foreground hover:text-foreground p-0.5"
              disabled={usageStore.isLoading}
              onclick={() => usageStore.refresh(true)}
              aria-label="Refresh AI usage"
            >
              <RotateCw size={12} class={usageStore.isLoading ? 'animate-gentle-spin' : ''} />
            </button>
          </div>
          {#if usageStore.isLoading && !usageStore.snapshot}
            <div class="py-2 text-center text-[10px] text-muted-foreground">Reading accounts…</div>
          {:else if selectedProviders.length}
            {#each selectedProviders as provider}
              <div class="flex items-center justify-between rounded-lg px-1.5 py-1 text-xs hover:bg-secondary/40">
                <div class="flex min-w-0 items-center gap-2">
                  <Bot size={13} class={provider.connected ? 'text-emerald-400' : 'text-muted-foreground'} />
                  <span class="truncate font-medium">{provider.name}</span>
                </div>
                <span class="ml-2 shrink-0 font-mono text-[10px] text-muted-foreground">{providerValue(provider)}</span>
              </div>
            {/each}
          {:else}
            <div class="py-2 text-center text-[10px] text-muted-foreground">Configure in Settings.</div>
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
                  {:else}<Boxes size={14} class="text-amber-400" />{/if}
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
      {/if}
    {/each}
  </div>

  <!-- Footer -->
  <div class="pt-3 border-t border-border/60 flex items-center justify-between gap-2">
    <div class="flex items-center gap-1.5 text-[11px] text-muted-foreground">
      <span>Last scan {formatTimeAgo(scan?.finished_at)}</span>
      <button
        type="button"
        disabled={scanStore.isScanning || scanStore.isCleaning}
        onclick={() => scanStore.runScan()}
        class="p-1 rounded-md hover:bg-secondary text-muted-foreground hover:text-foreground transition-colors disabled:opacity-50"
        title="Rescan storage"
      >
        <RotateCw size={11} class={scanStore.isScanning ? 'animate-gentle-spin' : ''} />
      </button>
    </div>
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

  {#if showResultModal && scanStore.lastCleanResult}
    <CleanResultModal result={scanStore.lastCleanResult} onClose={() => (showResultModal = false)} />
  {/if}
</div>
