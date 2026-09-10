<script lang="ts">
  interface Props {
    size?: number;
    class?: string;
  }

  let { size = 16, class: className = '' }: Props = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  xmlns="http://www.w3.org/2000/svg"
  aria-hidden="true"
  focusable="false"
  class="shrink-0 {className}"
>
  <circle
    cx="12"
    cy="12"
    r="9"
    pathLength="100"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-dasharray="68 32"
    class="animate-loading-dash"
  />
</svg>
