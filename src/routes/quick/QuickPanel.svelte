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
    if ((hasSection('cleanup') || hasSection('categories')) && scanStore.isStale()) {
      void scanStore.runScan();
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
    scanStore.selectAllSafe();
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

<div class="w-full h-full min-h-[500px] max-h-[540px] bg-background/95 backdrop-blur-xl border border-border/80 rounded-2xl flex flex-col justify-between p-4 select-none shadow-2xl text-foreground font-sans overflow-hidden">
  <div class="flex items-center justify-between pb-3 border-b border-border/60">
    <div class="flex items-center space-x-2">
      <div class="h-6 w-6 rounded-lg bg-primary text-primary-foreground flex items-center justify-center font-bold text-xs">Z</div>
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

  <div class="flex-1 overflow-y-auto py-3 space-y-3.5 pr-0.5">
    {#each settings.quick_panel_sections as section}
      {#if section === 'storage' && disk}
        <div class="space-y-1.5">
          <div class="flex justify-between text-xs font-medium"><span class="text-muted-foreground">Mac Storage</span><span class="font-mono text-foreground">{formatBytes(disk.used_bytes)} / {formatBytes(disk.total_bytes)}</span></div>
          <ProgressBar value={disk.percent_used} height="h-2" />
        </div>
      {:else if section === 'cleanup'}
        <div class="p-3.5 bg-secondary/50 border border-border/60 rounded-xl flex items-center justify-between">
          <div>
            <div class="text-[11px] text-muted-foreground font-medium uppercase tracking-wider">Can Clean (Safe)</div>
            <div class="text-xl font-bold font-mono text-foreground mt-0.5">{formatBytes(scanStore.safeSelectedBytes)}</div>
            <div class="text-[10px] text-muted-foreground mt-0.5">Last scan {formatTimeAgo(scan?.finished_at)}</div>
          </div>
          <Button variant="primary" size="sm" disabled={scanStore.isScanning || scanStore.isCleaning || scanStore.safeSelectedBytes === 0} onclick={handleCleanSafe} class="gap-1.5"><Trash2 size={13} /><span>Clean</span></Button>
        </div>
      {:else if section === 'ai_usage'}
        <div class="space-y-1.5 rounded-xl border border-border/60 bg-card/40 p-2.5">
          <div class="flex items-center justify-between px-1 pb-1">
            <div class="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground"><Sparkles size={12} class="text-violet-400" /> AI Usage</div>
            <button type="button" class="text-muted-foreground hover:text-foreground" disabled={usageStore.isLoading} onclick={() => usageStore.refresh(true)} aria-label="Refresh AI usage"><RotateCw size={12} class={usageStore.isLoading ? 'animate-spin' : ''} /></button>
          </div>
          {#if usageStore.isLoading && !usageStore.snapshot}
            <div class="py-3 text-center text-[10px] text-muted-foreground">Reading connected accounts…</div>
          {:else if selectedProviders.length}
            {#each selectedProviders as provider}
              <div class="flex items-center justify-between rounded-lg px-1.5 py-1.5 text-xs hover:bg-secondary/40">
                <div class="flex min-w-0 items-center gap-2"><Bot size={13} class={provider.connected ? 'text-emerald-400' : 'text-muted-foreground'} /><span class="truncate font-medium">{provider.name}</span></div>
                <span class="ml-2 shrink-0 font-mono text-[10px] text-muted-foreground">{providerValue(provider)}</span>
              </div>
            {/each}
          {:else}
            <div class="py-3 text-center text-[10px] text-muted-foreground">Choose providers in Settings.</div>
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
          <div class="py-6 text-center space-y-2"><RotateCw size={18} class="animate-spin mx-auto text-muted-foreground" /><p class="text-xs text-muted-foreground">Scanning development caches...</p></div>
        {/if}
      {:else if section === 'memory' && memory}
        <div class="pt-2 border-t border-border/60 flex items-center justify-between text-xs">
          <div class="flex items-center gap-1.5 text-muted-foreground"><Eye size={13} /><span>Memory ({memory.pressure})</span></div>
          <span class="font-mono text-foreground font-medium">{formatBytes(memory.used_bytes)} / {formatBytes(memory.total_bytes)}</span>
        </div>
      {/if}
    {/each}
  </div>

  <div class="pt-3 border-t border-border/60 flex items-center gap-2">
    <Button variant="outline" size="sm" disabled={scanStore.isScanning} onclick={() => scanStore.runScan()} class="flex-1 gap-1.5 text-xs"><RotateCw size={13} class={scanStore.isScanning ? 'animate-spin' : ''} /><span>{scanStore.isScanning ? 'Scanning...' : 'Scan Again'}</span></Button>
    <Button variant="secondary" size="sm" onclick={handleOpenDashboard} class="flex-1 gap-1.5 text-xs"><span>Open Zenith</span><ArrowRight size={13} /></Button>
  </div>

  {#if showResultModal && scanStore.lastCleanResult}
    <CleanResultModal result={scanStore.lastCleanResult} onClose={() => (showResultModal = false)} />
  {/if}
</div>
