<script lang="ts">
  import { onMount } from 'svelte';
  import type { CategoryResult, DashboardTab } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { memoryStore } from '../../lib/stores/memory.svelte';
  import { awakeStore } from '../../lib/stores/awake.svelte';
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import StorageView from './StorageView.svelte';
  import CategoryDetailView from './CategoryDetailView.svelte';
  import DockerView from './DockerView.svelte';
  import ModelsView from './ModelsView.svelte';
  import AiUsageView from './AiUsageView.svelte';
  import MemoryView from './MemoryView.svelte';
  import AwakeView from './AwakeView.svelte';
  import SettingsView from './SettingsView.svelte';
  import {
    HardDrive,
    Container,
    Boxes,
    Activity,
    Moon,
    Settings,
    Shield,
    ChartNoAxesCombined,
    Disc3,
  } from 'lucide-svelte';

  type Tab = DashboardTab | 'settings';

  let currentTab = $state<Tab>('storage');
  let selectedCategory = $state<CategoryResult | null>(null);
  let settings = $derived(settingsStore.settings);

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
      if (scanStore.isStale()) void scanStore.runScan();
    });
  });

  function selectTab(tab: Tab) {
    currentTab = tab;
    selectedCategory = null;
  }
</script>

<div class="flex h-screen w-screen bg-background text-foreground overflow-hidden font-sans select-none">
  <!-- Sidebar Navigation -->
  <aside
    class="w-56 shrink-0 bg-secondary/30 border-r border-border/70 flex flex-col justify-between p-3 pt-9"
  >
    <div class="space-y-6">
      <!-- Title & Branding -->
      <div class="px-2.5 flex items-center space-x-2.5">
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
        <div>
          <h1 class="text-sm font-bold tracking-tight text-foreground">Zenith</h1>
          <p class="text-[10px] text-muted-foreground font-mono">macOS Dev Manager</p>
        </div>
      </div>

      <!-- Navigation Tabs (Dynamically Ordered from Settings) -->
      <nav class="space-y-1">
        {#each (settings.dashboard_tabs || ['storage', 'docker', 'models', 'memory', 'usage', 'awake']) as tabId (tabId)}
          {@const tabInfo = tabDefs[tabId as DashboardTab]}
          {#if tabInfo}
            {@const Icon = tabInfo.icon}
            <button
              type="button"
              onclick={() => selectTab(tabId as Tab)}
              class="w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-colors {currentTab ===
              tabId && !selectedCategory
                ? 'bg-secondary text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'}"
            >
              <Icon size={15} />
              <span>{tabInfo.label}</span>
              {#if tabId === 'awake' && awakeStore.state.is_active}
                <span class="ml-auto w-1.5 h-1.5 rounded-full bg-amber-500 animate-pulse"></span>
              {/if}
            </button>
          {/if}
        {/each}

        <!-- Fixed Settings Tab -->
        <button
          type="button"
          onclick={() => selectTab('settings')}
          class="w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-colors {currentTab ===
          'settings'
            ? 'bg-secondary text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'}"
        >
          <Settings size={15} />
          <span>Settings</span>
        </button>
      </nav>
    </div>

    <!-- Safety Badge at bottom -->
    <div class="px-2.5 py-2 rounded-lg bg-card/60 border border-border/60 text-[11px] text-muted-foreground flex items-center gap-2">
      <Shield size={13} class="text-emerald-500 shrink-0" />
      <span class="truncate">TOCTOU Safety Guard</span>
    </div>
  </aside>

  <!-- Main Content Area -->
  <main class="flex-1 h-full overflow-y-auto p-8 pt-10">
    {#if selectedCategory}
      <CategoryDetailView
        categoryResult={selectedCategory}
        onBack={() => (selectedCategory = null)}
        onNavigateTab={(tab) => selectTab(tab)}
      />
    {:else if currentTab === 'storage'}
      <StorageView onSelectCategory={(cat) => (selectedCategory = cat)} />
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
  </main>
</div>
