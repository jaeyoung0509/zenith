<script lang="ts">
  interface Props {
    checked: boolean;
    disabled?: boolean;
    color?: string;
    ariaLabel: string;
    onchange?: (checked: boolean) => void;
  }

  let {
    checked = false,
    disabled = false,
    color = 'peer-checked:bg-primary',
    ariaLabel,
    onchange,
  }: Props = $props();

  function handleChange(event: Event) {
    if (disabled) return;
    const target = event.target as HTMLInputElement;
    onchange?.(target.checked);
  }
</script>

<label
  class="relative inline-flex items-center select-none {disabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer'}"
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
    class="w-9 h-5 bg-secondary peer-focus-visible:ring-2 peer-focus-visible:ring-ring rounded-full transition-colors duration-200 ease-out {color} relative shadow-xs"
  >
    <div
      class="absolute top-[2px] left-[2px] bg-white rounded-full h-4 w-4 shadow-xs transition-transform duration-200 ease-[cubic-bezier(0.16,1,0.3,1)] will-change-transform {checked ? 'translate-x-4' : 'translate-x-0'}"
    ></div>
  </div>
</label>
