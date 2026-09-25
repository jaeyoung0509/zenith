<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { systemMetricsStore } from '../../lib/stores/systemMetrics.svelte';
  import { memoryStore } from '../../lib/stores/memory.svelte';
  import SegmentedTabs from '../../lib/components/SegmentedTabs.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import CpuPanel from '../../lib/components/performance/CpuPanel.svelte';
  import MemoryPanel from '../../lib/components/performance/MemoryPanel.svelte';
  import BatteryPanel from '../../lib/components/performance/BatteryPanel.svelte';
  import { Activity, Battery, Cpu, MemoryStick } from '@lucide/svelte';

  type PerformanceTab = 'cpu' | 'memory' | 'battery';

  interface Props {
    initialTab?: PerformanceTab;
    /** The shell supplies its own tab selector; this page keeps its sections local. */
    onNavigateTab?: (route: string) => void;
  }

  let { initialTab = 'cpu' }: Props = $props();

  // The shell remounts this page per section, so the requested section is the
  // starting value; the strip owns the choice after that.
  let activeTab = $state<PerformanceTab>(untrack(() => initialTab));
  const panelId = $props.id();

  const tabs = [
    { id: 'cpu', label: 'CPU', icon: Cpu },
    { id: 'memory', label: 'Memory', icon: MemoryStick },
    { id: 'battery', label: 'Battery', icon: Battery },
  ] as const;

  const panelLabels: Record<PerformanceTab, string> = {
    cpu: 'CPU readings',
    memory: 'Memory readings',
    battery: 'Battery readings',
  };

  /**
   * One collector for the section on screen. The CPU and battery readings share
   * the system-metrics poller, the memory section keeps its own, and both stop
   * as soon as the tab changes or the window is hidden.
   */
  let systemPolling = false;
  let memoryPolling = false;

  function syncPollers(tab: PerformanceTab) {
    const visible = document.visibilityState === 'visible';
    const wantSystem = visible && tab !== 'memory';
    const wantMemory = visible && tab === 'memory';

    if (wantSystem !== systemPolling) {
      systemPolling = wantSystem;
      if (wantSystem) systemMetricsStore.startPolling(2500);
      else systemMetricsStore.stopPolling();
    }
    if (wantMemory !== memoryPolling) {
      memoryPolling = wantMemory;
      if (wantMemory) memoryStore.startPolling(2500);
      else memoryStore.stopPolling();
    }
  }

  onMount(() => {
    const onVisibilityChange = () => syncPollers(activeTab);
    syncPollers(activeTab);
    document.addEventListener('visibilitychange', onVisibilityChange);

    return () => {
      document.removeEventListener('visibilitychange', onVisibilityChange);
      if (systemPolling) {
        systemPolling = false;
        systemMetricsStore.stopPolling();
      }
      if (memoryPolling) {
        memoryPolling = false;
        memoryStore.stopPolling();
      }
    };
  });

  $effect(() => {
    syncPollers(activeTab);
  });

  function selectTab(id: string) {
    activeTab = id as PerformanceTab;
  }
</script>

<div class="space-y-5">
  <PageHeader
    title="Performance"
    subtitle="CPU, memory, and battery readings measured on this machine."
    icon={Activity}
  />

  <SegmentedTabs
    tabs={tabs}
    activeTab={activeTab}
    panelId={panelId}
    ariaLabel="Performance sections"
    onSelect={selectTab}
  />

  <div
    id={panelId}
    role="tabpanel"
    tabindex="0"
    aria-label={panelLabels[activeTab]}
    class="space-y-5 rounded-xl outline-none focus:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-offset-2"
  >
    {#if activeTab === 'cpu'}
      <CpuPanel />
    {:else if activeTab === 'memory'}
      <MemoryPanel />
    {:else}
      <BatteryPanel />
    {/if}
  </div>
</div>
