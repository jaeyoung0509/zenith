<script lang="ts">
  import { Check } from 'lucide-svelte';

  interface Props {
    checked: boolean;
    disabled?: boolean;
    ariaLabel: string;
    class?: string;
    onchange?: (checked: boolean) => void;
  }

  let {
    checked = false,
    disabled = false,
    ariaLabel,
    class: className = '',
    onchange,
  }: Props = $props();

  function handleChange(event: Event) {
    if (disabled) return;
    const target = event.target as HTMLInputElement;
    onchange?.(target.checked);
  }
</script>

<label
  class="relative inline-flex items-center justify-center select-none {disabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer'} {className}"
>
  <input
    type="checkbox"
    {checked}
    {disabled}
    aria-label={ariaLabel}
    onchange={handleChange}
    class="sr-only peer"
  />
  <div
    class="h-4 w-4 rounded-[4px] border transition-colors duration-150 flex items-center justify-center peer-focus-visible:ring-2 peer-focus-visible:ring-emerald-500/40 peer-focus-visible:ring-offset-1 peer-focus-visible:ring-offset-background {checked
      ? 'bg-emerald-500 border-emerald-500 text-white shadow-xs'
      : 'border-border/80 bg-secondary/40 hover:border-border text-transparent'}"
  >
    {#if checked}
      <Check size={11} strokeWidth={3} class="stroke-white" />
    {/if}
  </div>
</label>
