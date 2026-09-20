<script lang="ts">
  import { scanStore } from '../stores/scan.svelte';
  import { tauriOpenFullDiskAccessSettings } from '../utils/tauri';
  import Button from './Button.svelte';

  let settingsError = $state<string | null>(null);

  let fullDiskAccessGapCount = $derived(
    scanStore.lastScan?.gaps
      ?.filter((gap) => gap.kind === 'full_disk_access')
      .reduce((total, gap) => total + gap.count, 0) ?? 0
  );
  let hasFullDiskAccessGap = $derived(fullDiskAccessGapCount > 0);
  let hasSelectorTruncationGap = $derived(
    scanStore.lastScan?.gaps?.some((gap) => gap.kind === 'selector_truncated') ?? false
  );

  async function openFullDiskAccessSettings() {
    settingsError = null;
    try {
      await tauriOpenFullDiskAccessSettings();
    } catch (error) {
      settingsError = error instanceof Error ? error.message : String(error);
    }
  }

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
      {:else if scanStore.freshness === 'partial' && scanStore.cancelledScanNotice}
        {scanStore.cancelledScanNotice}
      {:else if scanStore.freshness === 'partial'}
        {#if hasFullDiskAccessGap}
          macOS denied access to {fullDiskAccessGapCount} {fullDiskAccessGapCount === 1 ? 'location' : 'locations'}. Grant Full Disk Access to Zenith, then scan again. Displayed totals are lower bounds (≥); incomplete items cannot be auto-cleaned.
        {:else if hasSelectorTruncationGap}
          Some locations were not inspected because the scan reached its bounded root limit. Displayed totals are lower bounds (≥); incomplete items cannot be auto-cleaned.
        {:else}
          Partial scan completed. Some locations could not be fully inspected, so displayed totals are lower bounds (≥). Incomplete items cannot be auto-cleaned.
        {/if}
      {:else if scanStore.freshness === 'unavailable'}
        {#if hasFullDiskAccessGap}
          Scan results are unavailable because macOS denied access to {fullDiskAccessGapCount} {fullDiskAccessGapCount === 1 ? 'location' : 'locations'}. Grant Full Disk Access to Zenith, then scan again. Cleaning is blocked until a scan succeeds.
        {:else}
          Scan results are unavailable because the configured locations could not be inspected. Cleaning is blocked until a scan succeeds.
        {/if}
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
      {#if settingsError}
        <span class="mt-1 block text-destructive">{settingsError}</span>
      {/if}
    </span>
    <span class="flex shrink-0 flex-wrap items-center gap-2">
      {#if hasFullDiskAccessGap && (scanStore.freshness === 'partial' || scanStore.freshness === 'unavailable')}
        <Button size="sm" variant="secondary" onclick={openFullDiskAccessSettings}>
          Open System Settings
        </Button>
      {/if}
      <Button size="sm" variant="outline" disabled={scanStore.isScanning || scanStore.isCleaning} onclick={() => scanStore.runScan()}>
        {scanStore.isScanning ? 'Scanning…' : 'Scan Again'}
      </Button>
    </span>
  </div>
{/if}
