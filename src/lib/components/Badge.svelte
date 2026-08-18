<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    variant?: 'default' | 'secondary' | 'outline' | 'success' | 'warning' | 'danger';
    class?: string;
    children?: Snippet;
  }

  let {
    variant = 'default',
    class: className = '',
    children,
  }: Props = $props();

  const variantStyles = {
    default: 'bg-primary text-primary-foreground',
    secondary: 'bg-secondary text-secondary-foreground',
    outline: 'border border-border text-foreground',
    success: 'bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20',
    warning: 'bg-amber-500/15 text-amber-600 dark:text-amber-400 border border-amber-500/20',
    danger: 'bg-rose-500/15 text-rose-600 dark:text-rose-400 border border-rose-500/20',
  };
</script>

<span
  class="inline-flex items-center px-2 py-0.5 rounded-full text-[10px] font-medium tracking-tight {variantStyles[
    variant
  ]} {className}"
>
  {#if children}
    {@render children()}
  {/if}
</span>
