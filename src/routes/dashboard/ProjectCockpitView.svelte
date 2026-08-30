<script lang="ts">
  import { onMount } from 'svelte';
  import { FolderGit2, RefreshCw } from 'lucide-svelte';
  import { agentActivityStore } from '../../lib/stores/agentActivity.svelte';
  import { usageStore } from '../../lib/stores/usage.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import Button from '../../lib/components/Button.svelte';
  import ProjectsPanel from '../../lib/components/ai-activity/ProjectsPanel.svelte';
  import ToolAdaptersPanel from '../../lib/components/ai-activity/ToolAdaptersPanel.svelte';
  import UsagePanel from '../../lib/components/ai-activity/UsagePanel.svelte';
  import ProjectDetailPanel from './ProjectDetailPanel.svelte';

  interface Props {
    onNavigateTab?: (tab: string) => void;
  }

  type AiActivitySubTab = 'usage' | 'projects' | 'adapters';

  const tabOrder: readonly AiActivitySubTab[] = ['usage', 'projects', 'adapters'];
  const tabLabels: Record<AiActivitySubTab, string> = {
    usage: 'Usage',
    projects: 'Projects',
    adapters: 'Tool Adapters',
  };

  let { onNavigateTab }: Props = $props();
  let activeSubTab = $state<AiActivitySubTab>('usage');
  let isMounted = $state(false);
  let selectedProject = $derived(agentActivityStore.selectedProject);
  let selectedProjectIntent = $derived(agentActivityStore.selectedProjectId !== null);
  let visibleSubTab = $derived<AiActivitySubTab>(
    selectedProject || selectedProjectIntent ? 'projects' : activeSubTab
  );
  let activeTabLabel = $derived(tabLabels[visibleSubTab]);
  let activeTabLoading = $derived(
    visibleSubTab === 'usage'
      ? usageStore.isLoading
      : visibleSubTab === 'projects'
        ? agentActivityStore.isLoading
        : agentActivityStore.isLoading || agentActivityStore.isIntegrationsLoading
  );

  const loadedTabs = new Set<AiActivitySubTab>();
  const loadingTabs = new Set<AiActivitySubTab>();

  onMount(() => {
    isMounted = true;
    void loadTab(visibleSubTab);
    return () => {
      isMounted = false;
    };
  });

  $effect(() => {
    const tab = visibleSubTab;
    if (isMounted) void loadTab(tab);
  });

  async function loadTab(tab: AiActivitySubTab) {
    if (loadedTabs.has(tab) || loadingTabs.has(tab)) return;
    loadingTabs.add(tab);

    try {
      if (tab === 'usage') {
        await usageStore.refreshIfStale();
      } else if (tab === 'projects') {
        // An already observed snapshot is the agent-activity cache. Refresh only on first load.
        if (!agentActivityStore.snapshot) await agentActivityStore.refresh();
      } else {
        // Adapters share the agent snapshot but have their own integration lookup boundary.
        if (!agentActivityStore.snapshot) await agentActivityStore.refresh();
        await agentActivityStore.fetchIntegrations();
      }
      loadedTabs.add(tab);
    } finally {
      loadingTabs.delete(tab);
    }
  }

  async function handleRefreshActive() {
    const tab = visibleSubTab;
    if (tab === 'usage') {
      await usageStore.refresh(true);
    } else if (tab === 'projects') {
      await agentActivityStore.refresh(true);
    } else {
      await Promise.all([
        agentActivityStore.refresh(true),
        agentActivityStore.fetchIntegrations(),
      ]);
    }
  }

  function tabId(tab: AiActivitySubTab) {
    return `ai-activity-tab-${tab}`;
  }

  function panelId(tab: AiActivitySubTab) {
    return `ai-activity-panel-${tab}`;
  }

  function selectSubTab(tab: AiActivitySubTab) {
    if (tab !== 'projects' && (selectedProject || selectedProjectIntent)) {
      agentActivityStore.selectProject(null);
    }
    activeSubTab = tab;
  }

  function handleTabKeydown(event: KeyboardEvent, currentTab: AiActivitySubTab) {
    const currentIndex = tabOrder.indexOf(currentTab);
    let nextIndex = currentIndex;

    if (event.key === 'ArrowRight') nextIndex = (currentIndex + 1) % tabOrder.length;
    else if (event.key === 'ArrowLeft') nextIndex = (currentIndex - 1 + tabOrder.length) % tabOrder.length;
    else if (event.key === 'Home') nextIndex = 0;
    else if (event.key === 'End') nextIndex = tabOrder.length - 1;
    else return;

    event.preventDefault();
    const nextTab = tabOrder[nextIndex];
    selectSubTab(nextTab);
    queueMicrotask(() => document.getElementById(tabId(nextTab))?.focus());
  }

  function handleBackToProjects() {
    agentActivityStore.selectProject(null);
    activeSubTab = 'projects';
  }
</script>

<div class="max-w-5xl space-y-6">
  <header class="flex items-start justify-between gap-4 border-b border-border/60 pb-4">
    <div class="flex items-center gap-3 min-w-0">
      <div class="h-9 w-9 shrink-0 rounded-lg bg-secondary text-foreground flex items-center justify-center">
        <FolderGit2 size={19} />
      </div>
      <div class="min-w-0">
        <div class="flex items-center gap-2">
          <h2 class="text-base font-semibold tracking-tight">AI Activity &amp; Projects</h2>
          <Badge variant="outline">Local only</Badge>
        </div>
        <p class="mt-0.5 text-xs text-muted-foreground">
          Connected AI account limits, active agent sessions, dev listeners, and local workspace storage.
        </p>
      </div>
    </div>
    <Button
      variant="outline"
      size="sm"
      disabled={activeTabLoading}
      ariaLabel={`Refresh ${activeTabLabel.toLowerCase()}`}
      title={`Refresh ${activeTabLabel.toLowerCase()}`}
      onclick={handleRefreshActive}
    >
      <RefreshCw size={13} class={activeTabLoading ? 'animate-spin' : ''} />
      {activeTabLoading ? `Refreshing ${activeTabLabel.toLowerCase()}` : `Refresh ${activeTabLabel.toLowerCase()}`}
    </Button>
  </header>

  <div
    role="tablist"
    aria-label="AI Activity sections"
    class="flex items-center gap-1 overflow-x-auto border-b border-border/60"
  >
    {#each tabOrder as tab}
      {@const isSelected = visibleSubTab === tab}
      <button
        type="button"
        id={tabId(tab)}
        role="tab"
        aria-selected={isSelected}
        aria-controls={panelId(tab)}
        tabindex={isSelected ? 0 : -1}
        class="relative shrink-0 px-3 py-2.5 text-xs transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring {isSelected
          ? 'font-semibold text-foreground after:absolute after:inset-x-2 after:bottom-0 after:h-0.5 after:bg-primary'
          : 'text-muted-foreground hover:text-foreground'}"
        onclick={() => selectSubTab(tab)}
        onkeydown={(event) => handleTabKeydown(event, tab)}
      >
        {tabLabels[tab]}
      </button>
    {/each}
  </div>

  {#if selectedProject}
    <div
      id="ai-activity-panel-projects"
      role="tabpanel"
      aria-labelledby="ai-activity-tab-projects"
      tabindex="0"
      class="outline-none focus-visible:ring-1 focus-visible:ring-ring"
    >
      <ProjectDetailPanel
        project={selectedProject}
        onBack={handleBackToProjects}
        onNavigateTab={onNavigateTab}
      />
    </div>
  {:else if visibleSubTab === 'usage'}
    <UsagePanel />
  {:else if visibleSubTab === 'projects'}
    <ProjectsPanel />
  {:else}
    <ToolAdaptersPanel />
  {/if}
</div>
