<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    class?: string;
    children?: Snippet;
    onclick?: () => void;
  }

  let { class: className = '', children, onclick }: Props = $props();
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
  class="bg-card text-card-foreground rounded-xl border border-border/80 p-4 shadow-sm {className}"
  {onclick}
>
  {#if children}
    {@render children()}
  {/if}
</div>
