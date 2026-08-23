<script lang="ts">
  interface Props {
    value: number; // 0 to 100 or current units
    max?: number;
    height?: string;
    class?: string;
    showPercent?: boolean;
    color?: string;
    animated?: boolean;
  }

  let {
    value = 0,
    max = 100,
    height = 'h-1.5',
    class: className = '',
    showPercent = false,
    color = 'bg-primary',
    animated = false,
  }: Props = $props();

  let percent = $derived(
    max > 0 ? Math.min(100, Math.max(0, (value / max) * 100)) : 0
  );
</script>

<div class="w-full space-y-1.5 {className}">
  {#if showPercent}
    <div class="flex justify-between text-meta text-muted-foreground font-mono">
      <span>Progress</span>
      <span>{Math.round(percent)}%</span>
    </div>
  {/if}
  <div class="w-full {height} bg-secondary/80 rounded-full overflow-hidden relative shadow-inner">
    <div
      class="{height} {color} rounded-full transition-[width,background-color] duration-500 ease-[cubic-bezier(0.16,1,0.3,1)] relative overflow-hidden"
      style="width: {percent}%;"
    >
      {#if animated}
        <div
          class="absolute inset-0 bg-gradient-to-r from-transparent via-white/25 to-transparent animate-shimmer"
        ></div>
      {/if}
    </div>
  </div>
</div>
