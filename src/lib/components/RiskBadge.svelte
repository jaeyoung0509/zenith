<script lang="ts">
  import type { RiskTier } from '../models/types';
  import Badge from './Badge.svelte';

  interface Props {
    risk: RiskTier;
  }

  let { risk }: Props = $props();

  const config = {
    safe: {
      label: 'Safe',
      variant: 'success' as const,
      tooltip: 'Safe cache or transient files.',
    },
    rebuild: {
      label: 'Rebuild',
      variant: 'warning' as const,
      tooltip: 'Auto-rebuildable or re-downloadable on demand.',
    },
    manual: {
      label: 'Manual',
      variant: 'danger' as const,
      tooltip: 'Stateful data requiring explicit manual review.',
    },
  };
</script>

<Badge variant={config[risk].variant} class="select-none font-mono">
  {config[risk].label}
</Badge>
