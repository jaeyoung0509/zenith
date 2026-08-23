<script lang="ts">
  import { onMount } from 'svelte';
  import { fade } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { prefersReducedMotion } from 'svelte/motion';
  import type { CategoryResult, DashboardTab } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { memoryStore } from '../../lib/stores/memory.svelte';
  import { awakeStore } from '../../lib/stores/awake.svelte';
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import StorageView from './StorageView.svelte';
  import StorageTools from './StorageTools.svelte';
  import CategoryDetailView from './CategoryDetailView.svelte';
  import DockerView from './DockerView.svelte';
  import ModelsView from './ModelsView.svelte';
  import AiUsageView from './AiUsageView.svelte';
  import MemoryView from './MemoryView.svelte';
  import AwakeView from './AwakeView.svelte';
  import SettingsView from './SettingsView.svelte';
  import LargeFilesView from './LargeFilesView.svelte';
  import ApplicationsView from './ApplicationsView.svelte';
  import { APP_VERSION, formatVersion } from '../../lib/utils/version';
  import { formatBytes } from '../../lib/utils/format';
  import { tauriStartWindowDrag } from '../../lib/utils/tauri';
  import Button from '../../lib/components/Button.svelte';
  import {
    Activity,
    Boxes,
    ChartNoAxesCombined,
    Container,
    ChevronsLeft,
    ChevronsRight,
    HardDrive,
    Moon,
    Settings,
    Shield,
  } from 'lucide-svelte';

  type Tab = DashboardTab | 'settings' | 'large-files' | 'applications';

  let currentTab = $state<Tab>('storage');
  let selectedCategory = $state<CategoryResult | null>(null);
  let settings = $derived(settingsStore.settings);
  let sidebarCollapsed = $derived(settings.sidebar_collapsed ?? false);
  let fadeDuration = $derived(prefersReducedMotion.current ? 0 : 140);

  const tabDefs: Partial<Record<DashboardTab, { label: string; icon: any }>> = {
    storage: { label: 'Storage', icon: HardDrive },
    docker: { label: 'Containers', icon: Container },
    models: { label: 'Local Models', icon: Boxes },
    memory: { label: 'Memory', icon: Activity },
    usage: { label: 'AI Usage', icon: ChartNoAxesCombined },
    awake: { label: 'Keep Awake', icon: Moon },
  };

  onMount(() => {
    memoryStore.refreshDisk();
    awakeStore.refresh();
    void scanStore.init().then(() => {
      if (scanStore.isStale()) {
        // Defer background revalidation so initial dashboard render is immediate
        if (typeof requestIdleCallback !== 'undefined') {
          requestIdleCallback(() => void scanStore.runScan(), { timeout: 1500 });
        } else {
          setTimeout(() => void scanStore.runScan(), 600);
        }
      }
    });
  });

  function selectTab(tab: Tab) {
    currentTab = tab;
    selectedCategory = null;
  }

  async function toggleSidebar() {
    await settingsStore.save({ sidebar_collapsed: !settingsStore.settings.sidebar_collapsed });
  }

  function handleWindowDrag(event: MouseEvent) {
    if (event.button !== 0) return;
    const target = event.target;
    if (target instanceof Element && target.closest('.no-drag')) return;
    void tauriStartWindowDrag().catch(() => undefined);
  }
</script>

<div class="flex h-screen w-screen bg-background text-foreground overflow-hidden font-sans select-none relative">
  <!-- Window drag region for macOS Overlay title bar -->
  <div
    class="titlebar-drag-region absolute top-0 left-0 right-0 h-7 z-30"
    aria-hidden="true"
    onmousedown={handleWindowDrag}
  ></div>
  <!-- Sidebar Navigation -->
  <aside
    class="{sidebarCollapsed ? 'w-16 p-2' : 'w-56 p-3'} shrink-0 bg-secondary/30 border-r border-border/70 flex flex-col justify-between pt-9 relative transition-[width,padding] duration-200"
  >
    <div class="space-y-6">
      <!-- Title & Branding -->
      <div class="flex items-center {sidebarCollapsed ? 'flex-col' : 'justify-between'} gap-2">
        <div
          class="{sidebarCollapsed ? 'px-0' : 'px-2.5'} flex items-center space-x-2.5 titlebar-drag-region"
          role="presentation"
          onmousedown={handleWindowDrag}
        >
          <svg class="h-6 w-6 rounded-lg shrink-0 shadow-sm" viewBox="0 0 1024 1024">
            <defs>
              <linearGradient id="dash-bg-grad" x1="160" y1="112" x2="864" y2="912" gradientUnits="userSpaceOnUse">
                <stop stop-color="#27272f"/>
                <stop offset="1" stop-color="#101014"/>
              </linearGradient>
            </defs>
            <rect width="1024" height="1024" rx="220" fill="url(#dash-bg-grad)"/>
            <path d="M292 300h466v116L486 650h282v116H266V650l270-234H292z" fill="#fff"/>
            <circle cx="758" cy="300" r="44" fill="#34d399"/>
          </svg>
          {#if !sidebarCollapsed}
            <span class="text-sm font-semibold tracking-tight text-foreground">Zenith</span>
          {/if}
        </div>

        <Button
          variant="ghost"
          size="icon"
          class="no-drag h-7 w-7 shrink-0 rounded-md border border-transparent bg-secondary/30 text-muted-foreground hover:border-border/70 hover:bg-secondary/80 hover:text-foreground"
          ariaLabel={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          title={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          onclick={toggleSidebar}
        >
          {#if sidebarCollapsed}
            <ChevronsRight size={14} strokeWidth={1.8} />
          {:else}
            <ChevronsLeft size={14} strokeWidth={1.8} />
          {/if}
        </Button>
      </div>

      <!-- Navigation Links -->
      <nav class="space-y-1 no-drag">
        {#each settings.dashboard_tabs ?? ['storage', 'docker', 'models', 'memory', 'usage', 'awake'] as tabId}
          {@const def = tabDefs[tabId as DashboardTab]}
          {#if def}
            <button
              type="button"
              onclick={() => selectTab(tabId as Tab)}
              aria-label={tabId === 'storage' && scanStore.reclaimableBytes > 0
                ? `${def.label}, ${formatBytes(scanStore.reclaimableBytes)} reclaimable`
                : def.label}
              title={sidebarCollapsed ? def.label : undefined}
              class="relative w-full flex items-center {sidebarCollapsed ? 'justify-center px-0' : 'gap-2.5 px-2.5'} py-1.5 rounded-lg text-xs font-medium transition-colors {currentTab ===
              tabId
                ? 'bg-secondary text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'}"
            >
              <def.icon size={15} />
              {#if !sidebarCollapsed}
                <span class="truncate">{def.label}</span>
              {/if}
              {#if tabId === 'storage' && scanStore.reclaimableBytes > 0}
                <span
                  class="{sidebarCollapsed
                    ? 'absolute right-1 top-1 h-1.5 w-1.5 rounded-full bg-success'
                    : 'ml-auto inline-flex items-center gap-1.5 whitespace-nowrap text-[10px] font-mono font-medium tracking-tight text-success/85'}"
                  title="Reclaimable storage available"
                >
                  {#if sidebarCollapsed}
                    <span class="sr-only">{formatBytes(scanStore.reclaimableBytes)} reclaimable</span>
                  {:else}
                    <span aria-hidden="true" class="h-1.5 w-1.5 rounded-full bg-success/90"></span>
                    {formatBytes(scanStore.reclaimableBytes)}
                  {/if}
                </span>
              {/if}
            </button>
          {/if}
        {/each}

        <!-- Fixed Settings Tab -->
        <button
          type="button"
          onclick={() => selectTab('settings')}
          aria-label="Settings"
          title={sidebarCollapsed ? 'Settings' : undefined}
          class="w-full flex items-center {sidebarCollapsed ? 'justify-center px-0' : 'gap-2.5 px-2.5'} py-1.5 rounded-lg text-xs font-medium transition-colors {currentTab ===
          'settings'
            ? 'bg-secondary text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'}"
        >
          <Settings size={15} />
          {#if !sidebarCollapsed}
            <span>Settings</span>
          {/if}
        </button>
      </nav>
    </div>

    <!-- Safety Badge & Version at bottom -->
    <div class="space-y-2 {sidebarCollapsed ? 'items-center' : ''}">
      <div
        class="{sidebarCollapsed ? 'justify-center px-0' : 'px-2.5'} py-2 rounded-lg bg-card/60 border border-border/60 text-[11px] text-muted-foreground flex items-center gap-2"
        title="Path, symlink and filesystem identity are verified immediately before deletion (TOCTOU protection)."
      >
        <Shield size={13} class="text-success shrink-0" />
        {#if !sidebarCollapsed}
          <span class="truncate">Protected cleanup</span>
        {/if}
      </div>
      {#if !sidebarCollapsed}
        <div class="px-2.5 flex items-center justify-between text-[10px] text-muted-foreground/60 font-mono select-none">
          <span>Zenith</span>
          <span>{formatVersion(APP_VERSION)}</span>
        </div>
      {/if}
    </div>
  </aside>

  <!-- Main Content Area with fluid native transition -->
  <main class="flex-1 h-full overflow-y-auto p-8 pt-10">
    {#key selectedCategory ? selectedCategory.category : currentTab}
      <div in:fade={{ duration: fadeDuration, easing: cubicOut }}>
        {#if selectedCategory}
          <CategoryDetailView
            categoryResult={selectedCategory}
            onBack={() => (selectedCategory = null)}
            onNavigateTab={(tab) => selectTab(tab)}
          />
        {:else if currentTab === 'storage'}
          <div class="space-y-6">
            <StorageTools
              onOpenLargeFiles={() => selectTab('large-files')}
              onOpenApplications={() => selectTab('applications')}
              onScanStorage={() => scanStore.runScan()}
              isScanning={scanStore.isScanning}
              isCleaning={scanStore.isCleaning}
            />
            <StorageView onSelectCategory={(cat) => (selectedCategory = cat)} />
          </div>
        {:else if currentTab === 'large-files'}
          <LargeFilesView onBack={() => selectTab('storage')} />
        {:else if currentTab === 'applications'}
          <ApplicationsView onBack={() => selectTab('storage')} />
        {:else if currentTab === 'docker'}
          <DockerView />
        {:else if currentTab === 'models'}
          <ModelsView />
        {:else if currentTab === 'usage'}
          <AiUsageView />
        {:else if currentTab === 'memory'}
          <MemoryView />
        {:else if currentTab === 'awake'}
          <AwakeView />
        {:else if currentTab === 'settings'}
          <SettingsView />
        {/if}
      </div>
    {/key}
  </main>
</div>
