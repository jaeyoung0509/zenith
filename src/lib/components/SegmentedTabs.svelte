<script lang="ts">
  import { handleSegmentedTabKeydown } from '../utils/segmentedTabs';

  interface TabItem {
    id: string;
    label: string;
    icon?: any;
    badge?: string | number;
  }

  interface Props {
    tabs: TabItem[];
    activeTab: string;
    onSelect: (id: string) => void;
    ariaLabel?: string;
    panelId?: string;
    class?: string;
  }

  let {
    tabs,
    activeTab,
    onSelect,
    ariaLabel = "Navigation sections",
    panelId,
    class: className = "",
  }: Props = $props();

  const instanceId = $props.id();
  let tablist: HTMLDivElement;

</script>

<div
  bind:this={tablist}
  role="tablist"
  aria-label={ariaLabel}
  class="inline-flex items-center gap-1 p-1 rounded-lg bg-secondary/50 border border-border/50 text-muted-foreground max-w-full overflow-x-auto {className}"
>
  {#each tabs as tab, index (tab.id)}
    {@const isSelected = activeTab === tab.id}
    <button
      type="button"
      id={instanceId + "-" + tab.id}
      role="tab"
      aria-selected={isSelected}
      aria-controls={panelId}
      tabindex={isSelected ? 0 : -1}
      onclick={() => onSelect(tab.id)}
      onkeydown={(event) => handleSegmentedTabKeydown(event, index, tabs, tablist, onSelect)}
      class="shrink-0 whitespace-nowrap inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium transition-[background-color,color] duration-140 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {isSelected
        ? "bg-card text-foreground shadow-xs font-semibold"
        : "text-muted-foreground hover:text-foreground hover:bg-secondary/70"}"
    >
      {#if tab.icon}
        <tab.icon size={14} class={isSelected ? "text-foreground" : "text-muted-foreground"} />
      {/if}
      <span>{tab.label}</span>
      {#if tab.badge !== undefined}
        <span
          class="ml-1 px-1.5 py-0.2 rounded-full text-micro font-mono {isSelected
            ? "bg-secondary text-foreground"
            : "bg-muted text-muted-foreground"}"
        >
          {tab.badge}
        </span>
      {/if}
    </button>
  {/each}
</div>
