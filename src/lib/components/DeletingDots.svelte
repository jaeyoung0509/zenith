<script lang="ts">
  interface Props {
    size?: 'xs' | 'sm' | 'md';
    color?: string;
    class?: string;
  }

  let { size = 'md', color = 'bg-current', class: className = '' }: Props = $props();

  let dotSize = $derived(size === 'xs' ? 'w-1 h-1' : size === 'sm' ? 'w-[5px] h-[5px]' : 'w-1.5 h-1.5');
</script>

<span class="inline-flex items-center gap-1 {className}" aria-label="Processing...">
  <span class="{dotSize} rounded-full {color} animate-bounce-dot-1"></span>
  <span class="{dotSize} rounded-full {color} animate-bounce-dot-2"></span>
  <span class="{dotSize} rounded-full {color} animate-bounce-dot-3"></span>
</span>
