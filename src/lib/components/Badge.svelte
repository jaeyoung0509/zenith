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
    success: 'bg-success/15 text-success border border-success/20',
    warning: 'bg-warning/15 text-warning border border-warning/20',
    danger: 'bg-destructive/15 text-destructive border border-destructive/20',
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
