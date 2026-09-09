<script lang="ts">
  import { onMount } from 'svelte';
  import { fade } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { prefersReducedMotion } from 'svelte/motion';
  import type { CategoryResult, DashboardRoute, DashboardTab } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { awakeStore } from '../../lib/stores/awake.svelte';
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import { platformCapabilitiesStore } from '../../lib/stores/platformCapabilities.svelte';
  import StorageView from './StorageView.svelte';
  import CategoryDetailView from './CategoryDetailView.svelte';
  import DockerView from './DockerView.svelte';
  import ModelsView from './ModelsView.svelte';
  import MemoryView from './MemoryView.svelte';
  import DevelopmentServersView from './DevelopmentServersView.svelte';
  import ProjectCockpitView from './ProjectCockpitView.svelte';
  import AiControlCenterView from './AiControlCenterView.svelte';
  import AwakeView from './AwakeView.svelte';
  import SettingsView from './SettingsView.svelte';
  import { APP_VERSION, formatVersion } from '../../lib/utils/version';
  import { formatBytes } from '../../lib/utils/format';
  import { normalizeDashboardTab } from '../../lib/utils/dashboardNavigation';
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
    Server,
    Settings,
    Shield,
    Sparkles,
  } from 'lucide-svelte';

  type Tab = DashboardRoute | 'large-files' | 'applications' | 'developer-artifacts' | 'disks';

  let currentTab = $state<Tab>('storage');
  let selectedCategory = $state<CategoryResult | null>(null);
  // SSR has no side effects, so render the existing dashboard shape for
  // snapshot tests. Browser mounts wait for the backend capability response
  // before mounting any platform-sensitive child route.
  let capabilitiesReady = $state(typeof window === 'undefined');
  let settings = $derived(settingsStore.settings);
  let sidebarCollapsed = $derived(settings.sidebar_collapsed ?? false);
  let fadeDuration = $derived(prefersReducedMotion.current ? 0 : 140);

  type DashboardCapability =
    | 'cleanup'
    | 'docker'
    | 'local_models'
    | 'memory_metrics'
    | 'development_ports'
    | 'ai_integrations'
    | 'keep_awake';

  const tabDefs: Partial<Record<DashboardTab, { label: string; icon: any; capability?: DashboardCapability }>> = {
    storage: { label: 'Storage', icon: HardDrive, capability: 'cleanup' },
    disk: { label: 'Disks', icon: HardDrive, capability: 'cleanup' },
    docker: { label: 'Containers', icon: Container, capability: 'docker' },
    models: { label: 'Local Models', icon: Boxes, capability: 'local_models' },
    memory: { label: 'Memory', icon: Activity, capability: 'memory_metrics' },
    development_servers: { label: 'Dev Servers', icon: Server, capability: 'development_ports' },
    projects: { label: 'AI Activity', icon: Sparkles, capability: 'ai_integrations' },
    ai_control: { label: 'AI Control', icon: Sparkles, capability: 'ai_integrations' },
    usage: { label: 'AI Usage', icon: ChartNoAxesCombined, capability: 'ai_integrations' },
    awake: { label: 'Keep Awake', icon: Moon, capability: 'keep_awake' },
  };

  const tabGroups: Record<DashboardTab, string> = {
    storage: 'Storage',
    disk: 'Storage',
    docker: 'Storage',
    models: 'Storage',
    memory: 'Runtime',
    development_servers: 'Runtime',
    awake: 'Runtime',
    projects: 'AI',
    ai_control: 'AI',
    usage: 'AI',
  };

  onMount(() => {
    let disposed = false;
    let stopFreshness: (() => void) | undefined;

    void platformCapabilitiesStore.load().then(() => {
      if (disposed) return;

      capabilitiesReady = true;
      const cleanupAvailable = platformCapabilitiesStore.isAvailable('cleanup');
      const awakeAvailable = platformCapabilitiesStore.isAvailable('keep_awake');

      // Do not mount or invoke platform-sensitive workflows until the backend
      // has told us that the corresponding adapter is available.
      if (cleanupAvailable) {
        stopFreshness = scanStore.observeFreshness();
        // Show the cached scan immediately; the freshness timer (#128)
        // auto-rescans within ~1s if it is stale, so no deferred scan here.
        void scanStore.init();
      }
      if (awakeAvailable) void awakeStore.refresh();

      // A persisted tab may have become unavailable after an upgrade or on a
      // different platform. Start on the first available tab instead of
      // mounting an unsupported route.
      const preferredTabs = settingsStore.settings.dashboard_tabs ?? [];
      const firstAvailable = preferredTabs.find((tab) => {
        const definition = tabDefs[tab as DashboardTab];
        return !!definition && (!definition.capability || platformCapabilitiesStore.isAvailable(definition.capability));
      });
      currentTab = normalizeDashboardTab(firstAvailable ?? 'settings') as Tab;
    });

    return () => {
      disposed = true;
      stopFreshness?.();
    };
  });

  function selectTab(tab: Tab | string) {
    if (!capabilitiesReady) return;
    const capability = tabDefs[tab as DashboardTab]?.capability;
    if (capability && !platformCapabilitiesStore.isAvailable(capability)) return;

    tab = normalizeDashboardTab(tab);
    if (tab === 'developer_artifacts' || tab === 'developer-artifacts') {
      currentTab = 'developer-artifacts';
    } else if (tab === 'large_files' || tab === 'large-files') {
      currentTab = 'large-files';
    } else if (tab === 'applications') {
      currentTab = 'applications';
    } else if (tab === 'disks') {
      currentTab = 'disks';
    } else if (tab === 'settings') {
      currentTab = 'settings';
    } else {
      currentTab = tab as Tab;
    }
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

<div class="flex h-screen w-full bg-background text-foreground overflow-hidden font-sans select-none relative">
  <!-- Window drag region for macOS Overlay title bar -->
  <div
    class="titlebar-drag-region absolute top-0 left-0 right-0 h-7 z-30"
    aria-hidden="true"
    onmousedown={handleWindowDrag}
  ></div>
  <!-- Sidebar Navigation -->
  <aside
    class="{sidebarCollapsed ? 'w-16 p-2' : 'w-56 p-3'} shrink-0 bg-secondary/30 border-r border-border/70 flex flex-col justify-between pt-9 relative transition-[width,padding] duration-150"
  >
    <div class="space-y-4 min-h-0 overflow-y-auto">
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
      <nav class="space-y-0.5 no-drag" aria-label="Main Navigation">
        {#each settings.dashboard_tabs ?? ['storage', 'docker', 'models', 'memory', 'development_servers', 'projects', 'awake'] as tabId, i}
          {@const def = tabDefs[tabId as DashboardTab]}
          {#if def}
            {@const capability = def.capability ? platformCapabilitiesStore.feature(def.capability) : null}
            {@const tabAvailable = capabilitiesReady && (!capability || platformCapabilitiesStore.isAvailable(def.capability!))}
            {@const currentGroup = tabGroups[tabId as DashboardTab]}
            {@const prevTabId = (settings.dashboard_tabs ?? [])[i - 1]}
            {@const prevGroup = prevTabId ? tabGroups[prevTabId as DashboardTab] : null}
            {@const showGroupHeader = !sidebarCollapsed && currentGroup && currentGroup !== prevGroup}
            {@const isTabActive = currentTab === tabId || (tabId === 'disk' && currentTab === 'disks') || (tabId === 'storage' && (currentTab === 'large-files' || currentTab === 'applications' || currentTab === 'developer-artifacts' || currentTab === 'disks'))}

            {#if showGroupHeader}
              <div class="px-2.5 {i === 0 ? 'pt-1' : 'pt-3'} pb-1 text-micro font-semibold uppercase tracking-wider text-muted-foreground/60 select-none">
                {currentGroup}
              </div>
            {:else if sidebarCollapsed && prevGroup && prevGroup !== currentGroup && i > 0}
              <div class="my-1.5 mx-2 h-px bg-border/40" role="separator"></div>
            {/if}

            <button
              type="button"
              onclick={() => selectTab(tabId as Tab)}
              disabled={!tabAvailable}
              aria-label={tabId === 'storage' && scanStore.reclaimableBytes > 0
                ? `${def.label}, ${formatBytes(scanStore.reclaimableBytes)} reclaimable`
                : def.label}
              title={tabAvailable ? (sidebarCollapsed ? def.label : undefined) : (capability?.reason ?? `${def.label} is unavailable`)}
              class="relative w-full flex items-center {sidebarCollapsed ? 'justify-center px-0' : 'gap-2.5 px-2.5'} py-1.5 rounded-lg text-xs font-medium transition-[background-color,color] duration-140 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {isTabActive
                ? 'bg-secondary text-foreground shadow-xs font-semibold'
                : tabAvailable
                  ? 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'
                  : 'text-muted-foreground/40 cursor-not-allowed'}"
            >
              {#if isTabActive && !sidebarCollapsed}
                <span class="absolute left-0 top-1.5 bottom-1.5 w-0.5 rounded-full bg-foreground" aria-hidden="true"></span>
              {/if}
              <def.icon size={15} />
              {#if !sidebarCollapsed}
                <span class="truncate">{def.label}</span>
              {/if}
              {#if tabId === 'storage' && scanStore.reclaimableBytes > 0}
                <span
                  class="{sidebarCollapsed
                    ? 'absolute right-1 top-1 h-1.5 w-1.5 rounded-full bg-success'
                    : 'ml-auto inline-flex items-center gap-1.5 whitespace-nowrap text-caption font-mono font-medium tracking-tight text-success/85'}"
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

        <!-- Group Separator for Settings -->
        <div class="pt-2" role="separator">
          {#if !sidebarCollapsed}
            <div class="px-2.5 pt-1 pb-1 text-micro font-semibold uppercase tracking-wider text-muted-foreground/60 select-none">
              Preferences
            </div>
          {:else}
            <div class="my-1 mx-2 h-px bg-border/40"></div>
          {/if}
        </div>

        <!-- Fixed Settings Tab -->
        <button
          type="button"
          onclick={() => selectTab('settings')}
          aria-label="Settings"
          title={sidebarCollapsed ? 'Settings' : undefined}
          class="relative w-full flex items-center {sidebarCollapsed ? 'justify-center px-0' : 'gap-2.5 px-2.5'} py-1.5 rounded-lg text-xs font-medium transition-[background-color,color] duration-140 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {currentTab ===
          'settings'
            ? 'bg-secondary text-foreground shadow-xs font-semibold'
            : 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'}"
        >
          {#if currentTab === 'settings' && !sidebarCollapsed}
            <span class="absolute left-0 top-1.5 bottom-1.5 w-0.5 rounded-full bg-foreground" aria-hidden="true"></span>
          {/if}
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
        class="{sidebarCollapsed ? 'justify-center px-0' : 'px-2.5'} py-2 rounded-lg bg-card/60 border border-border/60 text-meta text-muted-foreground flex items-center gap-2"
        title="Path, symlink and filesystem identity are verified immediately before deletion (TOCTOU protection)."
      >
        <Shield size={13} class="text-success shrink-0" />
        {#if !sidebarCollapsed}
          <span class="truncate">Protected cleanup</span>
        {/if}
      </div>
      {#if !sidebarCollapsed}
        <div class="px-2.5 flex items-center justify-between text-caption text-muted-foreground/60 font-mono select-none">
          <span>Zenith</span>
          <span>{formatVersion(APP_VERSION)}</span>
        </div>
      {/if}
    </div>
  </aside>

  <!-- Main Content Area with fluid native transition -->
  <main class="min-w-0 flex-1 h-full overflow-y-auto p-8 pt-10">
    {#if !capabilitiesReady}
      <div class="flex h-full items-center justify-center text-xs text-muted-foreground">Loading platform capabilities…</div>
    {:else}
      {#key selectedCategory ? selectedCategory.category : currentTab}
        <div in:fade={{ duration: fadeDuration, easing: cubicOut }}>
          {#if selectedCategory}
            <CategoryDetailView
              categoryResult={scanStore.lastScan?.categories.find((category) => category.category === selectedCategory?.category) ?? { ...selectedCategory, items: [], total_bytes: 0, safe_bytes: 0, rebuild_bytes: 0, manual_bytes: 0 }}
              onBack={() => (selectedCategory = null)}
              onNavigateTab={(tab) => selectTab(tab)}
            />
          {:else if currentTab === 'storage' || currentTab === 'large-files' || currentTab === 'applications' || currentTab === 'developer-artifacts' || currentTab === 'disks'}
            <StorageView
              initialTab={currentTab === 'storage' ? 'cleanup' : currentTab}
              onSelectCategory={(cat) => (selectedCategory = cat)}
            />
          {:else if currentTab === 'docker'}
            <DockerView />
          {:else if currentTab === 'models'}
            <ModelsView />
          {:else if currentTab === 'memory'}
            <MemoryView />
          {:else if currentTab === 'projects' || currentTab === 'usage'}
            <ProjectCockpitView onNavigateTab={(tab) => selectTab(tab)} />
          {:else if currentTab === 'ai_control'}
            <AiControlCenterView onNavigateTab={(tab) => selectTab(tab)} />
          {:else if currentTab === 'development_servers'}
            <DevelopmentServersView />
          {:else if currentTab === 'awake'}
            <AwakeView />
          {:else if currentTab === 'settings'}
            <SettingsView />
          {/if}
        </div>
      {/key}
    {/if}
  </main>
</div>
