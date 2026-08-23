<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    class?: string;
    children?: Snippet;
  }

  let { class: className = '', children }: Props = $props();
</script>

<div
  class="bg-card text-card-foreground rounded-xl border border-border/80 p-4 shadow-sm {className}"
>
  {#if children}
    {@render children()}
  {/if}
</div>
