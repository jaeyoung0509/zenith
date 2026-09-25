<script lang="ts">
  import { scanStore } from '../stores/scan.svelte';
  import { observedByteRange, summarizeCategory } from '../utils/cleanup';
  import { formatBytes } from '../utils/format';

  let scan = $derived(scanStore.lastScan);
  let summary = $derived(summarizeCategory(scan?.categories.flatMap(category => category.items) ?? []));
  let observed = $derived(observedByteRange(scan?.total_bytes ?? 0, scan?.ambiguous_overlap_bytes));
  let isCurrent = $derived(scanStore.freshness === 'fresh' || scanStore.freshness === 'partial');
  let estimateLabel = $derived(isCurrent ? 'Available to clean' : 'Last cleanup estimate');
</script>

<section class="storage-summary" aria-label="Storage scan summary">
  <div class="summary-primary">
    <p class="text-meta font-medium text-muted-foreground">{scan ? estimateLabel : 'Available to clean'}</p>
    <p class="mt-1 text-metric-lg font-semibold tracking-tight tabular-nums text-foreground">
      {scan ? summary.cleanable_bytes > 0 ? formatBytes(summary.cleanable_bytes) : summary.cleanable_count > 0 ? 'Amount varies' : formatBytes(0) : '—'}
    </p>
    <p class="mt-1 text-meta text-muted-foreground">
      {#if !scan}
        Scan known caches to find cleanup candidates.
      {:else if !isCurrent}
        Scan again to verify these results.
      {:else if summary.cleanable_count > 0}
        Review the items before cleaning.
      {:else}
        No cleanup candidates in this scan.
      {/if}
    </p>
  </div>
  <div class="summary-context">
    <p class="text-meta text-muted-foreground">Observed in scan</p>
    <p class="mt-1 text-sm font-medium font-mono tabular-nums text-foreground">
      {#if !scan}—
      {:else if observed.isAmbiguous}{formatBytes(observed.lower)}–{formatBytes(observed.upper)}
      {:else}{formatBytes(observed.upper)}{/if}
    </p>
    <p class="mt-1 text-meta text-muted-foreground">Includes items that must be kept.</p>
  </div>
</section>

<style>
  .storage-summary {
    display: grid;
    grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr);
    align-items: center;
    gap: 24px;
    padding: 0;
  }
  .summary-primary { padding-left: 16px; border-left: 3px solid hsl(var(--primary)); }
  .summary-context { border-left: 1px solid hsl(var(--border)); padding-left: 24px; }
  @container (max-width: 460px) {
    .storage-summary { grid-template-columns: minmax(0, 1fr); gap: 16px; }
    .summary-context { border: 0; padding-left: 19px; }
  }
</style>
