<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    variant?: 'primary' | 'secondary' | 'outline' | 'destructive' | 'ghost';
    size?: 'sm' | 'md' | 'lg' | 'icon';
    disabled?: boolean;
    id?: string;
    class?: string;
    onclick?: (e: MouseEvent) => void;
    ariaLabel?: string;
    title?: string;
    children?: Snippet;
  }

  let {
    variant = 'primary',
    size = 'md',
    disabled = false,
    id,
    class: className = '',
    onclick,
    ariaLabel,
    title,
    children,
  }: Props = $props();

  const variantStyles = {
    primary:
      'bg-primary text-primary-foreground hover:bg-primary/90 active:scale-[0.98] shadow-sm',
    secondary:
      'bg-secondary text-secondary-foreground hover:bg-secondary/80 active:scale-[0.98]',
    outline:
      'border border-border bg-transparent hover:bg-accent hover:text-accent-foreground active:scale-[0.98]',
    destructive:
      'bg-destructive text-destructive-foreground hover:bg-destructive/90 active:scale-[0.98] shadow-sm',
    ghost:
      'hover:bg-accent hover:text-accent-foreground active:scale-[0.98]',
  };

  const sizeStyles = {
    sm: 'h-7 px-2.5 text-xs rounded-md gap-1.5',
    md: 'h-9 px-3.5 text-xs font-medium rounded-lg gap-2',
    lg: 'h-10 px-4 text-sm font-medium rounded-lg gap-2',
    icon: 'h-8 w-8 rounded-lg flex items-center justify-center',
  };
</script>

<button
  type="button"
  {id}
  {disabled}
  {onclick}
  aria-label={ariaLabel}
  {title}
  class="inline-flex items-center justify-center whitespace-nowrap font-medium transition-[background-color,color,border-color,transform,opacity] duration-150 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-45 select-none {variantStyles[
    variant
  ]} {sizeStyles[size]} {className}"
>
  {#if children}
    {@render children()}
  {/if}
</button>
