<script lang="ts">
  interface Props {
    value: number; // 0 to 100
    max?: number;
    height?: string;
    class?: string;
    showPercent?: boolean;
    color?: string;
  }

  let {
    value = 0,
    max = 100,
    height = 'h-1.5',
    class: className = '',
    showPercent = false,
    color = 'bg-primary',
  }: Props = $props();

  let percent = $derived(Math.min(100, Math.max(0, (value / max) * 100)));
</script>

<div class="w-full space-y-1 {className}">
  {#if showPercent}
    <div class="flex justify-between text-[11px] text-muted-foreground font-mono">
      <span>Progress</span>
      <span>{Math.round(percent)}%</span>
    </div>
  {/if}
  <div class="w-full {height} bg-secondary/80 rounded-full overflow-hidden relative">
    <div
      class="{height} {color} rounded-full transition-all duration-300 ease-out relative overflow-hidden"
      style="width: {percent}%;"
    >
      <div
        class="absolute inset-0 bg-white/20 animate-[shimmer_2s_infinite] -skew-x-12"
      ></div>
    </div>
  </div>
</div>
