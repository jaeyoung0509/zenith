<script lang="ts">
  import { onMount } from 'svelte';
  import { fade } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { prefersReducedMotion } from 'svelte/motion';
  import type { CategoryResult, DashboardRoute, DashboardTab, PlatformCapabilities } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { awakeStore } from '../../lib/stores/awake.svelte';
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import { platformCapabilitiesStore } from '../../lib/stores/platformCapabilities.svelte';
  import { platformContextStore } from '../../lib/stores/platformContext.svelte';
  import PreviewModeIndicator from '../../lib/components/PreviewModeIndicator.svelte';
  import StorageView from './StorageView.svelte';
  import CategoryDetailView from './CategoryDetailView.svelte';
  import DockerView from './DockerView.svelte';
  import ModelsView from './ModelsView.svelte';
  import PerformanceView from './PerformanceView.svelte';
  import DevelopmentServersView from './DevelopmentServersView.svelte';
  import ProjectCockpitView from './ProjectCockpitView.svelte';
  import AiControlCenterView from './AiControlCenterView.svelte';
  import OverviewView from './OverviewView.svelte';
  import AwakeView from './AwakeView.svelte';
  import SettingsView from './SettingsView.svelte';
  import { APP_VERSION, formatVersion } from '../../lib/utils/version';
  import { formatBytes } from '../../lib/utils/format';
  import { DEFAULT_DASHBOARD_TABS, normalizeDashboardTab } from '../../lib/utils/dashboardNavigation';
  import { isTauri, tauriStartWindowDrag, tauriTakePendingNavigation } from '../../lib/utils/tauri';
  import Button from '../../lib/components/Button.svelte';
  import BrandIcon from '../../lib/components/BrandIcon.svelte';
  import Card from '../../lib/components/Card.svelte';
  import {
    Activity,
    AlertCircle,
    Boxes,
    ChartNoAxesCombined,
    Container,
    ChevronsLeft,
    ChevronsRight,
    Gauge,
    HardDrive,
    Moon,
    RotateCw,
    Server,
    Settings,
    Shield,
  } from '@lucide/svelte';

  /** Routes the shell can show: persisted routes plus the storage sub-views and
   * the Performance local tabs the Overview links into. */
  type Tab =
    | DashboardRoute
    | 'large-files'
    | 'applications'
    | 'developer-artifacts'
    | 'disks'
    | 'cpu'
    | 'battery';

  let currentTab = $state<Tab>('storage');
  let selectedCategory = $state<CategoryResult | null>(null);
  // SSR has no side effects, so render the existing dashboard shape for
  // snapshot tests. Browser mounts wait for the backend capability response
  // before mounting any platform-sensitive child route.
  let capabilitiesReady = $state(typeof window === 'undefined');
  // Derived from the store rather than a local flag so a failed query and an
  // unsupported platform can never be confused with each other.
  let capabilitiesFailed = $derived(
    platformCapabilitiesStore.error !== null && platformCapabilitiesStore.capabilities === null
  );
  let disposed = false;
  let workflowsMounted = false;
  let stopFreshness: (() => void) | undefined;
  let settings = $derived(settingsStore.settings);
  let sidebarCollapsed = $derived(settings.sidebar_collapsed ?? false);
  // Only the platform that actually draws an overlay title bar reserves the top
  // band or gets a drag strip; a native caption bar would otherwise show dead
  // pixels below it.
  let overlayTitleBar = $derived(platformContextStore.overlayTitleBar);
  let fadeDuration = $derived(prefersReducedMotion.current ? 0 : 140);

  type DashboardCapability = keyof Omit<PlatformCapabilities, 'platform'>;

  type TabDef = { label: string; icon: any };

  const tabDefs: Partial<Record<Tab, TabDef>> = {
    overview: { label: 'Overview', icon: Gauge },
    storage: { label: 'Storage', icon: HardDrive },
    disk: { label: 'Storage', icon: HardDrive },
    performance: { label: 'Performance', icon: Activity },
    memory: { label: 'Performance', icon: Activity },
    docker: { label: 'Containers', icon: Container },
    models: { label: 'Local Models', icon: Boxes },
    development_servers: { label: 'Dev Servers', icon: Server },
    projects: { label: 'AI Activity', icon: ChartNoAxesCombined },
    ai_control: { label: 'AI Activity', icon: ChartNoAxesCombined },
    usage: { label: 'AI Activity', icon: ChartNoAxesCombined },
    awake: { label: 'Keep Awake', icon: Moon },
  };

  /**
   * One capability per destination. A route whose adapter is missing stays
   * closed, and `memory` shares the Performance page's adapter because it is
   * that page's Memory detail.
   */
  const routeCapabilities: Record<string, DashboardCapability | null> = {
    overview: null,
    settings: null,
    storage: 'cleanup',
    'large-files': 'cleanup',
    applications: 'cleanup',
    'developer-artifacts': 'cleanup',
    disks: 'cleanup',
    performance: 'memory_metrics',
    memory: 'memory_metrics',
    cpu: 'cpu_metrics',
    battery: 'battery_metrics',
    docker: 'docker',
    models: 'local_models',
    development_servers: 'development_ports',
    projects: 'ai_integrations',
    usage: 'ai_integrations',
    ai_control: 'ai_integrations',
    awake: 'keep_awake',
  };

  // `null` keeps a destination ungrouped; the sidebar renders one heading per
  // contiguous group exactly as it renders one separator per group break.
  const tabGroups: Partial<Record<Tab, string | null>> = {
    overview: null,
    storage: null,
    disk: null,
    performance: null,
    memory: null,
    projects: null,
    ai_control: null,
    usage: null,
    docker: 'Tools',
    models: 'Tools',
    development_servers: 'Tools',
    awake: 'Tools',
  };

  /** Whether a destination can run on this platform right now. */
  function isRouteAvailable(route: string): boolean {
    if (!capabilitiesReady || capabilitiesFailed) return false;
    const capability = routeCapabilities[normalizeDashboardTab(route)] ?? null;
    return !capability || platformCapabilitiesStore.isAvailable(capability);
  }

  /**
   * Loads the backend capability matrix and, once it is known, mounts the
   * platform-sensitive workflows. Retrying after a failure repeats the same
   * path, so a transient IPC error cannot leave the dashboard permanently
   * gated and identical to an unsupported platform.
   */
  async function refreshCapabilities(force: boolean) {
    await platformCapabilitiesStore.load(force);
    if (disposed) return;

    capabilitiesReady = true;
    if (capabilitiesFailed || workflowsMounted) return;
    workflowsMounted = true;

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
    const preferredTabs = settingsStore.settings.dashboard_tabs ?? DEFAULT_DASHBOARD_TABS;
    const firstAvailable = preferredTabs.find((tab) => isRouteAvailable(tab));
    currentTab = normalizeDashboardTab(firstAvailable ?? 'settings') as Tab;
  }

  onMount(() => {
    void platformContextStore.load();
    void refreshCapabilities(false).then(async () => {
      if (disposed || !isTauri()) return;
      // The Quick Panel can ask for an exact destination; it is consumed once,
      // after the capability matrix is known, so a cold window load cannot lose
      // the request it was opened for.
      const route = await tauriTakePendingNavigation().catch(() => null);
      if (!disposed && route) selectTab(route);
    });

    return () => {
      disposed = true;
      stopFreshness?.();
    };
  });

  function selectTab(tab: Tab | string) {
    if (!isRouteAvailable(tab)) return;

    // The former `memory` route is the Memory detail of the Performance page,
    // so it keeps its own destination instead of collapsing into the default
    // CPU detail.
    if (tab === 'memory') {
      currentTab = 'memory';
      selectedCategory = null;
      return;
    }

    tab = normalizeDashboardTab(tab);
    if (tab === 'developer_artifacts' || tab === 'developer-artifacts') {
      currentTab = 'developer-artifacts';
    } else if (tab === 'large_files' || tab === 'large-files') {
      currentTab = 'large-files';
    } else if (tab === 'applications') {
      currentTab = 'applications';
    } else if (tab === 'disks') {
      currentTab = 'disks';
    } else if (tab === 'cpu' || tab === 'battery') {
      currentTab = tab;
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

<div class="dashboard-shell flex h-screen w-full bg-background text-foreground overflow-hidden font-sans select-none relative">
  {#if overlayTitleBar}
    <!-- Window drag region for the macOS overlay title bar -->
    <div
      class="titlebar-drag-region absolute top-0 left-0 right-0 h-7 z-30"
      aria-hidden="true"
      onmousedown={handleWindowDrag}
    ></div>
  {/if}
  <!-- Sidebar Navigation -->
  <aside
    class="liquid-sidebar {sidebarCollapsed ? 'w-16 p-2' : 'w-56 p-3'} shrink-0 border-r border-border flex flex-col justify-between {overlayTitleBar
      ? 'pt-9'
      : ''} relative transition-[width,padding] duration-150"
  >
    <div class="space-y-4 min-h-0 overflow-y-auto scroll-stable">
      <!-- Title & Branding -->
      <div class="flex items-center {sidebarCollapsed ? 'flex-col' : 'justify-between'} gap-2">
        <div
          class="{sidebarCollapsed ? 'px-0' : 'px-2.5'} flex items-center space-x-2.5 {overlayTitleBar
            ? 'titlebar-drag-region'
            : ''}"
          role="presentation"
          onmousedown={overlayTitleBar ? handleWindowDrag : undefined}
        >
          <BrandIcon identity="zenith" label="Zenith" size={24} />
          {#if !sidebarCollapsed}
            <span class="text-sm font-semibold tracking-tight text-foreground">Zenith</span>
          {/if}
        </div>

        <Button
          variant="ghost"
          size="icon"
          class="no-drag h-7 w-7 shrink-0 rounded-md border border-transparent text-muted-foreground hover:border-border hover:bg-card hover:text-foreground"
          ariaLabel={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          title={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          onclick={toggleSidebar}
        >
          {#if sidebarCollapsed}
            <ChevronsRight size={15} strokeWidth={1.8} />
          {:else}
            <ChevronsLeft size={15} strokeWidth={1.8} />
          {/if}
        </Button>
      </div>

      <!-- Navigation Links -->
      <nav class="space-y-0.5 no-drag" aria-label="Main Navigation">
        {#each settings.dashboard_tabs ?? DEFAULT_DASHBOARD_TABS as tabId, i}
          {@const def = tabDefs[tabId as DashboardTab]}
          {#if def}
            {@const capabilityName = routeCapabilities[tabId]}
            {@const capability = capabilityName ? platformCapabilitiesStore.feature(capabilityName) : null}
            {@const tabAvailable = isRouteAvailable(tabId)}
            {@const currentGroup = tabGroups[tabId as DashboardTab] ?? null}
            {@const prevTabId = (settings.dashboard_tabs ?? DEFAULT_DASHBOARD_TABS)[i - 1]}
            {@const prevGroup = prevTabId ? tabGroups[prevTabId as DashboardTab] ?? null : null}
            {@const showGroupHeader = !sidebarCollapsed && currentGroup && currentGroup !== prevGroup}
            {@const isTabActive = currentTab === tabId || (tabId === 'storage' && (currentTab === 'large-files' || currentTab === 'applications' || currentTab === 'developer-artifacts' || currentTab === 'disks'))}

            {#if showGroupHeader}
              <div class="px-2.5 {i === 0 ? 'pt-1' : 'pt-3'} pb-1 text-caption font-medium uppercase tracking-wide text-muted-foreground select-none">
                {currentGroup}
              </div>
            {:else if sidebarCollapsed && prevGroup && prevGroup !== currentGroup && i > 0}
              <div class="my-1.5 mx-2 h-px bg-border" role="separator"></div>
            {/if}

            <button
              type="button"
              onclick={() => selectTab(tabId as Tab)}
              disabled={!tabAvailable}
              aria-current={isTabActive ? 'page' : undefined}
              aria-label={tabId === 'storage' && scanStore.reclaimableBytes > 0
                ? `${def.label}, ${formatBytes(scanStore.reclaimableBytes)} reclaimable`
                : def.label}
              title={tabAvailable ? (sidebarCollapsed ? def.label : undefined) : (capability?.reason ?? `${def.label} is unavailable`)}
              class="relative w-full flex items-center {sidebarCollapsed ? 'justify-center px-0' : 'gap-2.5 px-2.5'} py-2 rounded-md text-body font-medium transition-[background-color,color] duration-140 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {isTabActive
                ? 'bg-accent/70 text-foreground'
                : tabAvailable
                  ? 'text-muted-foreground hover:text-foreground hover:bg-card'
                  : 'text-muted-foreground/50 cursor-not-allowed'}"
            >
              {#if isTabActive}<span aria-hidden="true" class="absolute left-0 inset-y-2 w-0.5 rounded-full bg-primary"></span>{/if}
              <def.icon size={17} strokeWidth={1.75} class="shrink-0" />
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
            <div class="px-2.5 pt-1 pb-1 text-caption font-medium uppercase tracking-wide text-muted-foreground select-none">
              Preferences
            </div>
          {:else}
            <div class="my-1 mx-2 h-px bg-border"></div>
          {/if}
        </div>

        <!-- Fixed Settings Tab -->
        <button
          type="button"
          onclick={() => selectTab('settings')}
          aria-current={currentTab === 'settings' ? 'page' : undefined}
          aria-label="Settings"
          title={sidebarCollapsed ? 'Settings' : undefined}
          class="relative w-full flex items-center {sidebarCollapsed ? 'justify-center px-0' : 'gap-2.5 px-2.5'} py-2 rounded-md text-body font-medium transition-[background-color,color] duration-140 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {currentTab ===
          'settings'
            ? 'bg-accent/70 text-foreground'
            : 'text-muted-foreground hover:text-foreground hover:bg-card'}"
        >
          {#if currentTab === 'settings'}<span aria-hidden="true" class="absolute left-0 inset-y-2 w-0.5 rounded-full bg-primary"></span>{/if}
          <Settings size={17} strokeWidth={1.75} class="shrink-0" />
          {#if !sidebarCollapsed}
            <span>Settings</span>
          {/if}
        </button>
      </nav>
    </div>

    <!-- Verification note & version at bottom -->
    <div class="space-y-2 {sidebarCollapsed ? 'items-center' : ''}">
      <div
        class="{sidebarCollapsed ? 'justify-center px-0' : 'px-2.5'} py-2 rounded-lg bg-card/70 border border-border text-meta text-muted-foreground flex items-center gap-2"
        title="Path, symlink and filesystem identity are re-derived immediately before deletion, and a refused item is reported instead of being removed."
      >
        <Shield size={14} class="text-muted-foreground shrink-0" />
        {#if !sidebarCollapsed}
          <span class="truncate">Verified on delete</span>
        {/if}
      </div>
      <PreviewModeIndicator compact={sidebarCollapsed} />
      {#if !sidebarCollapsed}
        <div class="px-2.5 flex items-center justify-between text-caption text-muted-foreground font-mono select-none">
          <span>Zenith</span>
          <span>{formatVersion(APP_VERSION)}</span>
        </div>
      {/if}
    </div>
  </aside>

  <!-- Main Content Area with fluid native transition -->
  <main class="@container min-w-0 flex-1 h-full overflow-y-auto scroll-stable {overlayTitleBar ? 'pt-10' : ''}">
    {#if !capabilitiesReady}
      <div class="flex h-full items-center justify-center text-body text-muted-foreground p-4 @2xl:p-6">Loading platform capabilities…</div>
    {:else if capabilitiesFailed}
      <div class="flex h-full items-center justify-center p-4 @2xl:p-6">
        <Card class="max-w-md p-6 space-y-3 text-center">
          <div class="mx-auto h-9 w-9 rounded-lg bg-destructive/10 text-destructive flex items-center justify-center">
            <AlertCircle size={18} />
          </div>
          <div class="space-y-1">
            <h2 class="text-sm font-semibold text-foreground">Platform capabilities unavailable</h2>
            <p class="text-body text-muted-foreground break-words">
              {platformCapabilitiesStore.error ?? 'Zenith could not read this platform\'s capability matrix from the backend.'}
            </p>
            <p class="text-meta text-muted-foreground">
              Tabs stay closed until the backend answers so no native action runs on an unverified platform.
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            disabled={platformCapabilitiesStore.isLoading}
            onclick={() => void refreshCapabilities(true)}
            class="mx-auto gap-1.5"
          >
            <RotateCw size={14} class={platformCapabilitiesStore.isLoading ? 'animate-gentle-spin' : ''} />
            <span>{platformCapabilitiesStore.isLoading ? 'Retrying…' : 'Retry'}</span>
          </Button>
        </Card>
      </div>
    {:else}
      <div class="p-4 @2xl:p-6">
        {#key selectedCategory ? selectedCategory.category : currentTab}
          <div in:fade={{ duration: fadeDuration, easing: cubicOut }}>
            {#if selectedCategory}
              <CategoryDetailView
                categoryResult={scanStore.lastScan?.categories.find((category) => category.category === selectedCategory?.category) ?? { ...selectedCategory, items: [], total_bytes: 0, safe_bytes: 0, rebuild_bytes: 0, manual_bytes: 0 }}
                onBack={() => (selectedCategory = null)}
                onNavigateTab={(tab) => selectTab(tab)}
              />
            {:else if currentTab === 'overview'}
              <OverviewView onNavigateTab={(tab) => selectTab(tab)} />
            {:else if currentTab === 'storage' || currentTab === 'large-files' || currentTab === 'applications' || currentTab === 'developer-artifacts' || currentTab === 'disks'}
              <StorageView
                initialTab={currentTab === 'storage' ? 'cleanup' : currentTab}
                onSelectCategory={(cat) => (selectedCategory = cat)}
              />
            {:else if currentTab === 'performance' || currentTab === 'memory' || currentTab === 'cpu' || currentTab === 'battery'}
              <PerformanceView
                initialTab={currentTab === 'memory' ? 'memory' : currentTab === 'battery' ? 'battery' : 'cpu'}
                onNavigateTab={(tab) => selectTab(tab)}
              />
            {:else if currentTab === 'docker'}
              <DockerView />
            {:else if currentTab === 'models'}
              <ModelsView />
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
      </div>
    {/if}
  </main>
</div>
