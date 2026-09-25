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

  async function openFullDiskAccessSettings() {
    settingsError = null;
    try {
      await tauriOpenFullDiskAccessSettings();
    } catch (error) {
      settingsError = error instanceof Error ? error.message : String(error);
    }
  }

</script>

{#if scanStore.freshness !== 'fresh' || scanStore.discovery.status !== 'exhausted'}
  <div class="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border bg-secondary p-3 text-meta" role="status">
    <span class="min-w-0 flex-1">
      {#if scanStore.freshness === 'refreshing'}
        Checking storage…
      {:else if scanStore.discovery.status === 'paused'}
        Checking storage…
      {:else if scanStore.discovery.status === 'stopped'}
        {scanStore.discovery.reason}
      {:else if scanStore.freshness === 'partial' && scanStore.cancelledScanNotice}
        {scanStore.cancelledScanNotice}
      {:else if scanStore.freshness === 'partial'}
        {#if hasFullDiskAccessGap}
          macOS could not read {fullDiskAccessGapCount} {fullDiskAccessGapCount === 1 ? 'location' : 'locations'}. Allow Full Disk Access to include them.
        {:else}
          Some locations could not be checked. Only verified items can be cleaned.
        {/if}
      {:else if scanStore.freshness === 'unavailable'}
        {#if hasFullDiskAccessGap}
          macOS blocked access to {fullDiskAccessGapCount} {fullDiskAccessGapCount === 1 ? 'location' : 'locations'}. Allow Full Disk Access, then scan again.
        {:else}
          Storage could not be checked. Try scanning again.
        {/if}
      {:else if scanStore.lastScan}
        Results are out of date. Scan again.
      {:else}
        Scan storage to find current cleanup candidates.
      {/if}
      {#if settingsError}
        <span class="mt-1 block text-meta text-destructive">{settingsError}</span>
      {/if}
    </span>
    <span class="flex shrink-0 flex-wrap items-center gap-2">
      {#if hasFullDiskAccessGap && (scanStore.freshness === 'partial' || scanStore.freshness === 'unavailable')}
        <Button size="sm" variant="secondary" onclick={openFullDiskAccessSettings}>
          Open System Settings
        </Button>
      {/if}
      {#if !scanStore.canContinue}
        <Button size="sm" variant="outline" disabled={scanStore.isScanning || scanStore.isCleaning} onclick={() => scanStore.runScan()}>
          {scanStore.isScanning ? 'Scanning…' : 'Scan Again'}
        </Button>
      {/if}
    </span>
  </div>
{/if}
