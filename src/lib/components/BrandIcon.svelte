<script lang="ts">
  import { brandFallbackGlyphs, brandIconProps, type BrandIconSize } from '../utils/brandIcons';

  interface Props {
    /** Registry identity or alias; falls back to `label` when absent. */
    identity?: string | null;
    /**
     * Product name shown next to the icon. That adjacent text owns the
     * accessible name, so the icon itself stays decorative; the label is only
     * consulted for an identity the caller could not name.
     */
    label: string;
    /** Identity slot in px; a brand minimum can raise the artwork past it. */
    size?: BrandIconSize;
    class?: string;
  }

  let { identity = null, label, size = 32, class: className = '' }: Props = $props();

  /**
   * Artwork drawn inside the slot. Tool rows use the 32 px slot with this 24 px
   * artwork (`DESIGN.md`, "Brand identity"); a brand's own minimum wins over
   * both, so Docker's mark is never drawn smaller than 24 px.
   */
  const ARTWORK_PX = 24;

  const icon = $derived(brandIconProps(identity || label));
  const FallbackGlyph = $derived(brandFallbackGlyphs[icon.fallbackGlyph]);
  const artwork = $derived(Math.max(icon.minSizePx, Math.min(size, ARTWORK_PX)));
  const slot = $derived(Math.max(size, artwork));
</script>

<span
  class="inline-flex shrink-0 items-center justify-center {className}"
  style="width: {slot}px; height: {slot}px;"
>
  {#if icon.assetSrc}
    <!--
      Reviewed artwork, drawn exactly as published: no filter, tint, clip or
      stretch. Only the height is pinned, so the intrinsic aspect ratio holds,
      and the image is decorative because the adjacent label names the product.
    -->
    <img
      src={icon.assetSrc}
      alt=""
      aria-hidden="true"
      draggable="false"
      class="max-w-full object-contain"
      style="height: {artwork}px; width: auto;"
    />
  {:else}
    <FallbackGlyph size={artwork} strokeWidth={1.75} aria-hidden="true" />
  {/if}
</span>
