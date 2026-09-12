<script lang="ts">
  import { scanStore } from '../stores/scan.svelte';
  import Button from './Button.svelte';

  /** How much of the last scan could not be measured, as an operator summary. */
  let measurementGaps = $derived.by(() => {
    const scan = scanStore.lastScan;
    if (!scan) return null;
    const parts: string[] = [];
    const skipped = scan.skipped_entry_count ?? 0;
    const incomplete = scan.incomplete_item_count ?? 0;
    if (skipped > 0) parts.push(`${skipped} entries skipped`);
    // An `Unavailable` item was not measured at all, so the summary says what
    // is true of both it and a partially measured one.
    if (incomplete > 0) parts.push(`${incomplete} items not fully measured`);
    return parts.length > 0 ? parts.join(' · ') : null;
  });
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
      {#if measurementGaps && scanStore.freshness !== 'refreshing'}
        <span class="mt-1 block font-mono text-caption text-muted-foreground">
          {measurementGaps}
        </span>
      {/if}
    </span>
    <Button size="sm" variant="outline" disabled={scanStore.isScanning || scanStore.isCleaning} onclick={() => scanStore.runScan()}>
      {scanStore.isScanning ? 'Scanning…' : 'Scan Again'}
    </Button>
  </div>
{/if}
