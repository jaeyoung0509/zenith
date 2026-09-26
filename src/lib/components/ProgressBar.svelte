<script lang="ts">
  interface Props {
    value: number; // 0 to 100 or current units
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
  <div class="w-full {height} meter-track rounded-full overflow-hidden" role="presentation">
    <div
      class="{height} {color === 'bg-primary' || color === 'bg-ai' ? 'meter-fill' : color} rounded-full transition-[width] duration-300 ease-[cubic-bezier(0.16,1,0.3,1)]"
      style="width: {percent}%;"
    ></div>
  </div>
</div>
