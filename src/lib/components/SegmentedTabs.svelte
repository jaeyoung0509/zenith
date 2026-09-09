<script lang="ts">
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
    class?: string;
  }

  let {
    tabs,
    activeTab,
    onSelect,
    ariaLabel = "Navigation sections",
    class: className = "",
  }: Props = $props();

  function handleKeydown(event: KeyboardEvent, currentIndex: number) {
    let nextIndex = currentIndex;
    if (event.key === "ArrowRight") {
      nextIndex = (currentIndex + 1) % tabs.length;
    } else if (event.key === "ArrowLeft") {
      nextIndex = (currentIndex - 1 + tabs.length) % tabs.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = tabs.length - 1;
    } else {
      return;
    }

    event.preventDefault();
    const nextTab = tabs[nextIndex];
    if (nextTab) {
      onSelect(nextTab.id);
      const button = document.getElementById("tab-" + nextTab.id);
      button?.focus();
    }
  }
</script>

<div
  role="tablist"
  aria-label={ariaLabel}
  class="inline-flex items-center gap-1 p-1 rounded-lg bg-secondary/50 border border-border/50 text-muted-foreground {className}"
>
  {#each tabs as tab, index (tab.id)}
    {@const isSelected = activeTab === tab.id}
    <button
      type="button"
      id={"tab-" + tab.id}
      role="tab"
      aria-selected={isSelected}
      tabindex={isSelected ? 0 : -1}
      onclick={() => onSelect(tab.id)}
      onkeydown={(e) => handleKeydown(e, index)}
      class="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium transition-[background-color,color,box-shadow] duration-140 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {isSelected
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
