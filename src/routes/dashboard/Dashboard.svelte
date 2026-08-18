<script lang="ts">
  import { onMount } from 'svelte';
  import type { CategoryResult } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import { memoryStore } from '../../lib/stores/memory.svelte';
  import { awakeStore } from '../../lib/stores/awake.svelte';
  import StorageView from './StorageView.svelte';
  import DiskView from './DiskView.svelte';
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
  } from 'lucide-svelte';

  type Tab = 'storage' | 'disk' | 'docker' | 'models' | 'usage' | 'memory' | 'awake' | 'settings';

  let currentTab = $state<Tab>('storage');
  let selectedCategory = $state<CategoryResult | null>(null);

  onMount(() => {
    memoryStore.refreshDisk();
    awakeStore.refresh();
    if (!scanStore.lastScan) {
      scanStore.runScan();
    }
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
        <div class="h-6 w-6 rounded-lg bg-primary text-primary-foreground flex items-center justify-center font-bold text-xs">
          Z
        </div>
        <div>
          <h1 class="text-sm font-bold tracking-tight text-foreground">Zenith</h1>
          <p class="text-[10px] text-muted-foreground font-mono">macOS Dev Manager</p>
        </div>
      </div>

      <!-- Navigation Tabs -->
      <nav class="space-y-1">
        <button
          type="button"
          onclick={() => selectTab('disk')}
          class="w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-colors {currentTab ===
          'disk'
            ? 'bg-secondary text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'}"
        >
          <HardDrive size={15} />
          <span>Disks</span>
        </button>

        <button
          type="button"
          onclick={() => selectTab('storage')}
          class="w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-colors {currentTab ===
            'storage' && !selectedCategory
            ? 'bg-secondary text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'}"
        >
          <HardDrive size={15} />
          <span>Storage</span>
        </button>

        <button
          type="button"
          onclick={() => selectTab('docker')}
          class="w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-colors {currentTab ===
          'docker'
            ? 'bg-secondary text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'}"
        >
          <Container size={15} />
          <span>Containers</span>
        </button>

        <button
          type="button"
          onclick={() => selectTab('models')}
          class="w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-colors {currentTab ===
          'models'
            ? 'bg-secondary text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'}"
        >
          <Boxes size={15} />
          <span>Local Models</span>
        </button>

        <button
          type="button"
          onclick={() => selectTab('memory')}
          class="w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-colors {currentTab ===
          'memory'
            ? 'bg-secondary text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'}"
        >
          <Activity size={15} />
          <span>Memory</span>
        </button>

        <button
          type="button"
          onclick={() => selectTab('usage')}
          class="w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-colors {currentTab ===
          'usage'
            ? 'bg-secondary text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'}"
        >
          <ChartNoAxesCombined size={15} />
          <span>AI Usage</span>
        </button>

        <button
          type="button"
          onclick={() => selectTab('awake')}
          class="w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-xs font-medium transition-colors {currentTab ===
          'awake'
            ? 'bg-secondary text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground hover:bg-secondary/50'}"
        >
          <Moon size={15} />
          <span>Keep Awake</span>
          {#if awakeStore.state.is_active}
            <span class="ml-auto w-1.5 h-1.5 rounded-full bg-amber-500 animate-pulse"></span>
          {/if}
        </button>

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
      />
    {:else if currentTab === 'storage'}
      <StorageView onSelectCategory={(cat) => (selectedCategory = cat)} />
    {:else if currentTab === 'docker'}
      <DockerView />
    {:else if currentTab === 'disk'}
      <DiskView onReviewCategory={(category) => (selectedCategory = category)} />
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
