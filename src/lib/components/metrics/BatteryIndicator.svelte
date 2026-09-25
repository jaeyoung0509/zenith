<script lang="ts">
  import type { BatteryChargeState } from '../../models/types';

  interface Props {
    /** Charge percentage 0-100, or null when the platform does not report one. */
    percent: number | null;
    chargeState: BatteryChargeState;
    class?: string;
  }

  let { percent, chargeState, class: className = '' }: Props = $props();

  const WIDTH = 26;
  const HEIGHT = 13;
  const NUB_WIDTH = 2.5;

  let fillRatio = $derived(percent == null ? 0 : Math.min(1, Math.max(0, percent / 100)));
  let fillWidth = $derived(fillRatio * (WIDTH - 4));

  /**
   * A low, actively discharging battery is the only caution state worth amber:
   * a plugged-in machine that happens not to be charging is not a warning.
   */
  let fillClass = $derived(
    chargeState === 'discharging' && percent != null && percent <= 20
      ? 'fill-warning'
      : 'fill-foreground'
  );
</script>

<svg
  class="{className}"
  viewBox="0 0 {WIDTH + NUB_WIDTH} {HEIGHT}"
  width={WIDTH + NUB_WIDTH}
  height={HEIGHT}
  aria-hidden="true"
  focusable="false"
>
  <rect
    x="0.5"
    y="0.5"
    width={WIDTH - 1}
    height={HEIGHT - 1}
    rx="3.5"
    fill="none"
    stroke="hsl(var(--border-strong))"
    stroke-width="1"
  />
  <rect x={WIDTH} y={(HEIGHT - 5) / 2} width={NUB_WIDTH} height="5" rx="1.2" fill="hsl(var(--border-strong))" />
  {#if fillRatio > 0}
    <rect
      x="2.5"
      y="2.5"
      width={fillWidth}
      height={HEIGHT - 5}
      rx="1.8"
      class={fillClass}
    />
  {/if}
  {#if chargeState === 'charging'}
    <!-- Lightning is shown only while the platform reports actual charging. -->
    <path d="M14.2 2.2 L10.6 6.8 H13.2 L11.6 10.8 L15.4 6.2 H12.8 Z" class="fill-background" />
  {/if}
</svg>
