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
            <linearGradient id="zenith-glow" x1="0%" y1="0%" x2="100%" y2="100%">
              <stop offset="0%" stop-color="#3b82f6" />
              <stop offset="100%" stop-color="#10b981" />
            </linearGradient>
          </defs>
          <rect width="1024" height="1024" rx="224" fill="#090d16" />
          <path
            d="M 256 320 L 768 320 L 320 704 L 768 704"
            fill="none"
            stroke="url(#zenith-glow)"
            stroke-width="112"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
        <span class="text-sm font-semibold tracking-tight text-foreground">Zenith</span>
      </div>

      <!-- Navigation Links -->
      <nav class="space-y-1">
        {#each settings.dashboard_tabs ?? ['storage', 'docker', 'models', 'memory', 'usage', 'awake'] as tabId}
          {@const def = tabDefs[tabId as DashboardTab]}
          {#if def}
            <button
              type="button"
              onclick={() => selectTab(tabId as Tab)}
              class="w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-colors {currentTab ===
              tabId
                ? 'bg-secondary text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'}"
            >
              <def.icon size={15} />
              <span>{def.label}</span>
              {#if tabId === 'storage' && scanStore.reclaimableBytes > 0}
                <span class="ml-auto flex h-1.5 w-1.5 rounded-full bg-emerald-500"></span>
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
    <div
      class="px-2.5 py-2 rounded-lg bg-card/60 border border-border/60 text-[11px] text-muted-foreground flex items-center gap-2"
      title="Path, symlink and filesystem identity are verified immediately before deletion (TOCTOU protection)."
    >
      <Shield size={13} class="text-emerald-500 shrink-0" />
      <span class="truncate">Protected cleanup</span>
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
