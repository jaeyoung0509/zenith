<script lang="ts">
  import { onMount } from 'svelte';

  let scene: SVGSVGElement;
  let motionVisible = $state(false);
  onMount(() => {
    let intersects = false;
    const updateVisibility = () => { motionVisible = intersects && !document.hidden; };
    const observer = new IntersectionObserver(([entry]) => {
      intersects = entry.isIntersecting;
      updateVisibility();
    });
    observer.observe(scene);
    document.addEventListener('visibilitychange', updateVisibility);
    return () => {
      observer.disconnect();
      document.removeEventListener('visibilitychange', updateVisibility);
    };
  });
  // Fixed geometry keeps this decorative scene stable across renders and themes.
  const id = $props.id();
  const dust = Array.from({ length: 96 }, (_, index) => {
    const angle = index * 2.399963;
    const radius = 70 + ((index * 29) % 43);
    return {
      x: 120 + Math.cos(angle) * radius,
      y: 100 + Math.sin(angle) * radius * 0.57,
      radius: index % 13 === 0 ? 1.15 : 0.45,
      opacity: 0.2 + (index % 5) * 0.1,
      warm: index % 7 === 0,
    };
  });
</script>

<svg bind:this={scene} class="celestial-scene" style:--scene-play-state={motionVisible ? 'running' : 'paused'} viewBox="0 0 240 200" aria-hidden="true" focusable="false">
  <defs>
    <radialGradient id={`${id}-halo`}>
      <stop offset="0.45" stop-color="hsl(var(--cosmic-ice))" stop-opacity="0.2" />
      <stop offset="1" stop-color="hsl(var(--cosmic-ice))" stop-opacity="0" />
    </radialGradient>
    <radialGradient id={`${id}-planet`} cx="76%" cy="24%" r="84%">
      <stop offset="0" stop-color="hsl(var(--cosmic-water))" />
      <stop offset="0.48" stop-color="hsl(var(--cosmic-surface))" />
      <stop offset="1" stop-color="hsl(var(--cosmic-night))" />
    </radialGradient>
    <radialGradient id={`${id}-shade`} cx="87%" cy="20%" r="89%">
      <stop offset="0.15" stop-color="hsl(var(--cosmic-night))" stop-opacity="0" />
      <stop offset="0.7" stop-color="hsl(var(--cosmic-night))" stop-opacity="0.25" />
      <stop offset="1" stop-color="hsl(var(--cosmic-night))" stop-opacity="0.96" />
    </radialGradient>
    <clipPath id={`${id}-disc`}><circle cx="120" cy="100" r="59" /></clipPath>
    <filter id={`${id}-clouds`} x="0" y="0" width="100%" height="100%" color-interpolation-filters="sRGB">
      <feTurbulence type="fractalNoise" baseFrequency="0.045 0.09" numOctaves="3" seed="23" />
      <feColorMatrix type="saturate" values="0" />
      <feComponentTransfer><feFuncA type="linear" slope="0.34" /></feComponentTransfer>
      <feComposite in2="SourceGraphic" operator="in" />
    </filter>
  </defs>
  <ellipse cx="120" cy="103" rx="111" ry="90" fill={`url(#${id}-halo)`} />
  <g transform="rotate(-24 120 100)">
    <ellipse cx="120" cy="100" rx="104" ry="58" fill="none" stroke="hsl(var(--cosmic-line))" stroke-width="0.6" opacity="0.24" />
    <g class="orbit-dust">
    {#each dust as star}
      <circle cx={star.x} cy={star.y} r={star.radius} opacity={star.opacity}
        fill={star.warm ? 'hsl(var(--cosmic-warm))' : 'hsl(var(--cosmic-line))'} />
    {/each}
    </g>
  </g>
  <circle cx="120" cy="100" r="60" fill="hsl(var(--cosmic-ice))" opacity="0.3" />
  <circle cx="120" cy="100" r="59" fill={`url(#${id}-planet)`} />
  <g clip-path={`url(#${id}-disc)`}>
    <g class="planet-clouds">
      <circle cx="120" cy="100" r="59" fill="hsl(var(--cosmic-highlight))" filter={`url(#${id}-clouds)`} />
    </g>
    <circle cx="120" cy="100" r="59" fill={`url(#${id}-shade)`} />
  </g>
  <path d="M 131 42 A 59 59 0 0 1 177 85" fill="none" stroke="hsl(var(--cosmic-highlight))" stroke-width="1" opacity="0.75" />
  <circle cx="186" cy="53" r="1.6" fill="hsl(var(--cosmic-line))" />
  <circle cx="186" cy="53" r="4" fill="hsl(var(--cosmic-ice))" opacity="0.16" />
</svg>

<style>
  .orbit-dust, .planet-clouds {
    transform-origin: 120px 100px;
    animation: orbit 48s linear infinite;
    animation-play-state: var(--scene-play-state, paused);
  }
  .planet-clouds { animation-duration: 36s; animation-direction: reverse; }
  @keyframes orbit { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce), (prefers-reduced-transparency: reduce) {
    .orbit-dust, .planet-clouds { animation: none; }
  }
</style>
