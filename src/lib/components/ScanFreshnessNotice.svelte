<script lang="ts">
  import { scanStore } from '../stores/scan.svelte';
  import Button from './Button.svelte';
</script>

{#if scanStore.freshness !== 'fresh'}
  <div class="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-warning/30 bg-warning/10 p-3 text-xs" role="status">
    <span class="min-w-0 flex-1">
      {#if scanStore.freshness === 'refreshing'}
        {#if scanStore.lastScanTrigger === 'auto'}
          Auto-refreshing scan. Previous amounts are historical until this finishes.
        {:else}
          Refreshing scan. Previous amounts are historical until this finishes.
        {/if}
      {:else if scanStore.freshness === 'partial'}
        Partial scan completed. Some locations could not be fully inspected, so displayed totals are lower bounds (≥). Incomplete items cannot be auto-cleaned.
      {:else if scanStore.freshness === 'unavailable'}
        Scan results are unavailable because the configured locations could not be inspected. Cleaning is blocked until a scan succeeds.
      {:else if scanStore.lastScan}
        Results are out of date. Scan again, then review the new selection before cleaning.
      {:else}
        Scan storage to find current cleanup candidates.
      {/if}
    </span>
    <Button size="sm" variant="outline" disabled={scanStore.isScanning || scanStore.isCleaning} onclick={() => scanStore.runScan()}>
      {scanStore.isScanning ? 'Scanning…' : 'Scan Again'}
    </Button>
  </div>
{/if}
