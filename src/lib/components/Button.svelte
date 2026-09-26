<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    variant?: 'primary' | 'secondary' | 'outline' | 'destructive' | 'ghost';
    size?: 'xs' | 'sm' | 'md' | 'lg' | 'icon';
    motion?: 'scale' | 'paint';
    disabled?: boolean;
    id?: string;
    class?: string;
    onclick?: (e: MouseEvent) => void;
    ariaLabel?: string;
    ariaExpanded?: boolean;
    ariaControls?: string;
    title?: string;
    children?: Snippet;
  }

  let {
    variant = 'primary',
    size = 'md',
    motion = 'scale',
    disabled = false,
    id,
    class: className = '',
    onclick,
    ariaLabel,
    ariaExpanded,
    ariaControls,
    title,
    children,
  }: Props = $props();

  const variantStyles = {
    primary:
      'bg-action text-action-foreground hover:bg-action/85',
    secondary:
      'bg-secondary text-secondary-foreground hover:bg-secondary/80',
    outline:
      'border border-border-strong bg-transparent hover:bg-accent hover:text-accent-foreground',
    destructive:
      'bg-destructive text-destructive-foreground hover:bg-destructive/90',
    ghost:
      'hover:bg-accent hover:text-accent-foreground',
  };

  const motionStyles = {
    scale: 'transition-[background-color,color,border-color,transform,opacity] duration-100 active:scale-[0.98]',
    paint: 'transition-[background-color,color,border-color] duration-100',
  };

  // Main-window controls sit at 32–36 px; icon-only targets never drop below 28 px.
  const sizeStyles = {
    xs: 'h-7 px-2 text-caption font-medium rounded-lg gap-1',
    sm: 'h-8 px-2.5 text-meta font-medium rounded-lg gap-1.5',
    md: 'h-9 px-3.5 text-body font-medium rounded-lg gap-2',
    lg: 'h-9 px-4 text-body font-medium rounded-lg gap-2',
    icon: 'h-8 w-8 rounded-lg flex items-center justify-center',
  };
</script>

<button
  type="button"
  {id}
  {disabled}
  {onclick}
  aria-label={ariaLabel}
  aria-expanded={ariaExpanded}
  aria-controls={ariaControls}
  {title}
  class="inline-flex items-center justify-center whitespace-nowrap font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 disabled:pointer-events-none disabled:opacity-45 select-none {motionStyles[
    motion
  ]} {variantStyles[
    variant
  ]} {sizeStyles[size]} {className}"
>
  {#if children}
    {@render children()}
  {/if}
</button>
