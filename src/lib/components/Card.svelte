<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    /**
     * `card` is the white page surface, `subtle` the secondary grouped surface,
     * and `plain` a transparent wrapper for rows that already sit in one surface.
     */
    surface?: 'card' | 'subtle' | 'plain';
    class?: string;
    children?: Snippet;
    onclick?: () => void;
  }

  let { surface = 'card', class: className = '', children, onclick }: Props = $props();

  const surfaceStyles = {
    card: 'bg-card text-card-foreground border border-border',
    subtle: 'bg-secondary text-secondary-foreground border border-border',
    plain: 'bg-transparent border border-transparent',
  };
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
  class="rounded-xl p-4 {surfaceStyles[surface]} {className}"
  {onclick}
>
  {#if children}
    {@render children()}
  {/if}
</div>
