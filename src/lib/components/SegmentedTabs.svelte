<script lang="ts">
  import { handleSegmentedTabKeydown } from '../utils/segmentedTabs';

  interface TabItem {
    id: string;
    label: string;
    icon?: any;
    badge?: string | number;
  }

  interface Props {
    tabs: readonly TabItem[];
    activeTab: string;
    onSelect: (id: string) => void;
    ariaLabel?: string;
    panelId?: string;
    class?: string;
    appearance?: 'segmented' | 'underline';
  }

  let {
    tabs,
    activeTab,
    onSelect,
    ariaLabel = "Navigation sections",
    panelId,
    class: className = "",
    appearance = 'segmented',
  }: Props = $props();

  const instanceId = $props.id();
  let tablist: HTMLDivElement;

</script>

<div
  bind:this={tablist}
  role="tablist"
  aria-label={ariaLabel}
  class="text-muted-foreground max-w-full {appearance === 'underline' ? 'section-tabs flex flex-wrap items-center gap-x-4 border-b border-border' : 'inline-flex items-center gap-1 p-1 rounded-lg border border-border bg-card overflow-x-auto'} {className}"
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
      class="shrink-0 whitespace-nowrap inline-flex h-8 items-center gap-1.5 px-3 rounded-md text-meta font-medium transition-[background-color,color] duration-140 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {appearance === 'underline' ? 'section-tab' : ''} {isSelected
        ? "bg-accent text-foreground font-semibold"
        : "text-muted-foreground hover:text-foreground hover:bg-secondary"}"
    >
      {#if tab.icon}
        <tab.icon size={14} class={isSelected ? "text-foreground" : "text-muted-foreground"} />
      {/if}
      <span>{tab.label}</span>
      {#if tab.badge !== undefined}
        <span
          class="ml-1 px-1.5 py-0.5 rounded-full text-micro font-mono {isSelected
            ? "bg-card text-foreground"
            : "bg-secondary text-muted-foreground"}"
        >
          {tab.badge}
        </span>
      {/if}
    </button>
  {/each}
</div>

<style>
  .section-tabs { row-gap: 4px; }
  .section-tab {
    height: 40px;
    padding: 0 2px;
    border-radius: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    cursor: pointer;
  }
  .section-tab[aria-selected='true'] {
    color: hsl(var(--primary));
    border-bottom-color: hsl(var(--primary));
  }
  .section-tab:focus-visible { outline: 2px solid hsl(var(--ring)); outline-offset: -3px; }
</style>
