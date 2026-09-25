<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    variant?: 'default' | 'secondary' | 'outline' | 'success' | 'warning' | 'danger' | 'destructive' | 'ai';
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
    outline: 'border border-border-strong text-foreground',
    success: 'bg-success/10 text-success border border-success/25',
    warning: 'bg-warning/10 text-warning border border-warning/25',
    danger: 'bg-destructive/10 text-destructive border border-destructive/25',
    destructive: 'bg-destructive/10 text-destructive border border-destructive/25',
    ai: 'bg-ai/10 text-ai border border-ai/25',
  };
</script>

<span
  class="inline-flex items-center px-2 py-0.5 rounded-md text-caption font-medium tracking-tight {variantStyles[
    variant
  ]} {className}"
>
  {#if children}
    {@render children()}
  {/if}
</span>
