<script lang="ts">
  import { tick } from 'svelte';
  import type {
    LargeFileItem,
    LargeFileScanEvent,
    LargeFileScanResult,
    TrashPlanPreview,
    TrashResult,
  } from '../../lib/models/types';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import { formatBytes, formatCountdown, formatTimeAgo, ttlRemaining } from '../../lib/utils/format';
  import {
    LARGE_FILE_DEFAULT_THRESHOLD_BYTES,
    LARGE_FILE_ROOTS,
    clampLargeFileThreshold,
    largeFileKindLabel,
    selectedLargeFileBytes,
  } from '../../lib/utils/storageManagement';
  import {
    tauriCancelLargeFileScan,
    tauriExecuteTrashPlan,
    tauriPrepareLargeFileTrash,
    tauriRevealInFinder,
    tauriStartLargeFileScan,
  } from '../../lib/utils/tauri';
  import {
    AlertCircle,
    ArrowLeft,
    CheckSquare,
    FileSearch,
    FolderOpen,
    RefreshCw,
    ShieldCheck,
    Square,
    Trash2,
    X,
  } from 'lucide-svelte';

  interface Props {
    onBack: () => void;
  }

  let { onBack }: Props = $props();

  const MIB = 1024 * 1024;

  let selectedRoots = $state<string[]>(LARGE_FILE_ROOTS.map((root) => root.id));
  let thresholdMb = $state(LARGE_FILE_DEFAULT_THRESHOLD_BYTES / MIB);
  let items = $state<LargeFileItem[]>([]);
  // Streaming scans can report thousands of matches; a Set keeps duplicate
  // suppression O(1) per event instead of rescanning the whole array.
  let seenItemIds = new Set<string>();
  let selectedIds = $state<string[]>([]);
  let scanResult = $state<LargeFileScanResult | null>(null);
  let plan = $state<TrashPlanPreview | null>(null);
  let trashResult = $state<TrashResult | null>(null);
  let isScanning = $state(false);
  let isPreparing = $state(false);
  let isExecuting = $state(false);
  let activeScanId = $state<string | null>(null);
  let activeRoot = $state('');
  let entriesScanned = $state(0);
  let matchesFound = $state(0);
  let error = $state<string | null>(null);
  let selectedBytes = $derived(selectedLargeFileBytes(items, selectedIds));
  let thresholdBytes = $derived(clampLargeFileThreshold(thresholdMb * MIB));
  let now = $state(Date.now());

  let remainingSecs = $derived(
    plan ? ttlRemaining(plan.expires_at, now) : 0
  );
  let isExpiringSoon = $derived(remainingSecs > 0 && remainingSecs <= 60);
  let isExpired = $derived(plan ? remainingSecs === 0 : false);
  let expiryActionFocused = $state(false);

  $effect(() => {
    if (!isExpired) {
      expiryActionFocused = false;
      return;
    }
    if (expiryActionFocused) return;
    expiryActionFocused = true;
    void tick().then(() => document.getElementById('large-files-expiry-action')?.focus());
  });

  $effect(() => {
    if (!plan) return;
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });
  function resetReview() {
    plan = null;
    trashResult = null;
  }
  function toggleRoot(root: string) {
    selectedRoots = selectedRoots.includes(root)
      ? selectedRoots.filter((candidate) => candidate !== root)
      : [...selectedRoots, root];
  }

  function toggleItem(id: string) {
    selectedIds = selectedIds.includes(id)
      ? selectedIds.filter((candidate) => candidate !== id)
      : [...selectedIds, id];
    resetReview();
  }

  function selectAll() {
    selectedIds = items.map((item) => item.id);
    resetReview();
  }

  function deselectAll() {
    selectedIds = [];
    resetReview();
  }

  function handleScanEvent(event: LargeFileScanEvent) {
    switch (event.type) {
      case 'started':
        activeScanId = event.scan_id;
        break;
      case 'root_started':
        activeRoot = event.root;
        break;
      case 'progress':
        activeRoot = event.root;
        entriesScanned = event.entries_scanned;
        matchesFound = event.matches_found;
        break;
      case 'item_found':
        if (!seenItemIds.has(event.item.id)) {
          seenItemIds.add(event.item.id);
          items.push(event.item);
          matchesFound = items.length;
        }
        break;
      case 'root_finished':
        activeRoot = event.root;
        break;
      case 'finished':
        scanResult = event.result;
        items = event.result.items;
        entriesScanned = event.result.entries_scanned;
        matchesFound = event.result.items.length;
        break;
      case 'cancelled':
        activeScanId = event.scan_id;
        break;
    }
  }

  async function scanFiles() {
    if (selectedRoots.length === 0) {
      error = 'Select at least one folder to scan.';
      return;
    }

    isScanning = true;
    error = null;
    plan = null;
    trashResult = null;
    scanResult = null;
    selectedIds = [];
    items = [];
    seenItemIds.clear();
    activeScanId = null;
    activeRoot = '';
    entriesScanned = 0;
    matchesFound = 0;

    try {
      const result = await tauriStartLargeFileScan(
        { roots: selectedRoots, min_size_bytes: thresholdBytes },
        handleScanEvent
      );
      scanResult = result;
      items = result.items;
      entriesScanned = result.entries_scanned;
      matchesFound = result.items.length;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      isScanning = false;
      activeScanId = null;
      activeRoot = '';
    }
  }

  async function cancelScan() {
    if (!activeScanId) return;
    try {
      await tauriCancelLargeFileScan(activeScanId);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  async function reviewTrash() {
    if (!scanResult || selectedIds.length === 0) return;
    isPreparing = true;
    error = null;
    trashResult = null;
    try {
      plan = await tauriPrepareLargeFileTrash(scanResult.scan_id, selectedIds);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      isPreparing = false;
    }
  }

  async function executeTrash() {
    if (!plan) return;
    isExecuting = true;
    error = null;
    try {
      const result = await tauriExecuteTrashPlan(plan.id);
      trashResult = result;
      const movedIds = new Set(
        result.items.filter((item) => item.success).map((item) => item.item_id)
      );
      items = items.filter((item) => !movedIds.has(item.id));
      selectedIds = selectedIds.filter((id) => !movedIds.has(id));
      plan = null;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      isExecuting = false;
    }
  }
</script>

<div class="space-y-5">
  <div class="flex items-start gap-3">
    <Button variant="ghost" size="icon" onclick={onBack} ariaLabel="Back to Storage">
      <ArrowLeft size={16} />
    </Button>
    <div class="min-w-0 flex-1">
      <div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <h1 class="text-xl font-semibold tracking-tight">Large Files</h1>
        <div class="flex shrink-0 items-center gap-1.5 text-[11px] text-muted-foreground">
          <ShieldCheck size={14} class="text-emerald-500" />
          <span>Identity rechecked before Trash</span>
        </div>
      </div>
      <p class="mt-1 text-xs text-muted-foreground">
        Inspect large personal files. Nothing is selected automatically and deletion always goes through Trash.
      </p>
    </div>
  </div>

  <Card class="p-5 space-y-4">
    <div class="grid grid-cols-1 lg:grid-cols-[1fr_auto] gap-5 items-end">
      <div class="space-y-3">
        <div>
          <div class="text-xs font-medium mb-2">Folders</div>
          <div class="flex flex-wrap gap-2">
            {#each LARGE_FILE_ROOTS as root}
              <label class="inline-flex items-center gap-2 px-3 py-2 rounded-lg border border-border/70 bg-secondary/20 text-xs cursor-pointer hover:bg-secondary/40">
                <input
                  type="checkbox"
                  checked={selectedRoots.includes(root.id)}
                  onchange={() => toggleRoot(root.id)}
                  class="accent-emerald-500"
                />
                <span>{root.label}</span>
              </label>
            {/each}
          </div>
        </div>

        <div class="flex flex-wrap items-center gap-3">
          <label for="large-file-threshold" class="text-xs font-medium">Minimum size</label>
          <select
            id="large-file-threshold"
            bind:value={thresholdMb}
            disabled={isScanning}
            class="h-8 rounded-lg border border-border bg-background px-2.5 text-xs"
          >
            <option value={100}>100 MB</option>
            <option value={500}>500 MB</option>
            <option value={1024}>1 GB</option>
            <option value={2048}>2 GB</option>
            <option value={5120}>5 GB</option>
          </select>
          <span class="text-[11px] text-muted-foreground">
            Backend safety floor: 100 MB
          </span>
        </div>
      </div>

      <div class="flex items-center gap-2">
        {#if isScanning && activeScanId}
          <Button variant="outline" size="md" onclick={cancelScan} class="gap-1.5">
            <X size={14} />
            Cancel
          </Button>
        {/if}
        <Button
          variant="primary"
          size="md"
          onclick={scanFiles}
          disabled={isScanning || selectedRoots.length === 0}
          class="gap-1.5 min-w-[120px]"
        >
          <RefreshCw size={14} class={isScanning ? 'animate-gentle-spin' : ''} />
          {isScanning ? 'Scanning…' : 'Scan Files'}
        </Button>
      </div>
    </div>

    {#if isScanning}
      <div class="pt-3 border-t border-border/60 flex flex-wrap items-center justify-between gap-2 text-[11px] text-muted-foreground">
        <span>Scanning {activeRoot || 'selected folders'}…</span>
        <span class="font-mono">{entriesScanned.toLocaleString()} entries · {matchesFound} matches</span>
      </div>
    {/if}
  </Card>

  {#if error}
    <div class="p-3.5 rounded-xl bg-destructive/15 border border-destructive/30 text-destructive flex items-center gap-2.5 text-xs">
      <AlertCircle size={16} class="shrink-0" />
      <span>{error}</span>
    </div>
  {/if}

  {#if trashResult}
    <Card class={`p-4 ${trashResult.failed_count + trashResult.skipped_count > 0 ? 'border-amber-500/30 bg-amber-500/5' : 'border-emerald-500/30 bg-emerald-500/5'}`}>
      <div class="flex items-center justify-between gap-3 text-xs">
        <span class={`font-medium ${trashResult.failed_count + trashResult.skipped_count > 0 ? 'text-amber-500' : 'text-emerald-500'}`}>
          Moved {trashResult.moved_count} item{trashResult.moved_count === 1 ? '' : 's'} to Trash
          {#if trashResult.failed_count + trashResult.skipped_count > 0}
            · {trashResult.failed_count + trashResult.skipped_count} not moved
          {/if}
        </span>
        <span class="font-mono text-muted-foreground">{formatBytes(trashResult.moved_allocated_size)}</span>
      </div>
    </Card>
  {/if}

  {#if scanResult?.truncated}
    <div class="p-3.5 rounded-xl bg-amber-500/10 border border-amber-500/30 text-amber-500 flex items-center gap-2.5 text-xs">
      <AlertCircle size={16} class="shrink-0" />
      <span>More than 10,000 files matched. Results show the 10,000 largest files.</span>
    </div>
  {/if}

  {#if plan}
    <Card class={`p-5 space-y-3 ${isExpired ? 'border-red-500/40 bg-red-500/5' : isExpiringSoon ? 'border-amber-500/50 bg-amber-500/10' : 'border-amber-500/30 bg-amber-500/5'}`}>
      <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
        <div>
          <div class="text-sm font-semibold flex items-center gap-2">
            Trash review ready
            <span class={`text-[10px] px-1.5 py-0.5 rounded font-mono border ${isExpired ? 'bg-red-500/15 text-red-400 border-red-500/30' : isExpiringSoon ? 'bg-amber-500/15 text-amber-500 border-amber-500/30' : 'bg-secondary text-muted-foreground border-border'}`}>
              {formatCountdown(remainingSecs)}
            </span>
          </div>
          <p class="text-xs text-muted-foreground mt-1">
            {plan.item_count} selected item{plan.item_count === 1 ? '' : 's'} · {formatBytes(plan.allocated_size)} allocated
          </p>
        </div>
        <div class="flex items-center gap-2">
          <Button variant="ghost" size="sm" onclick={() => (plan = null)}>Cancel</Button>
          <Button variant="destructive" size="md" onclick={executeTrash} disabled={isExecuting || isExpired} class="gap-1.5" title={isExpired ? 'Plan expired — prepare again' : ''}>
            <Trash2 size={14} />
            {isExecuting ? 'Moving…' : isExpired ? 'Expired' : 'Move to Trash'}
          </Button>
        </div>
      </div>
      {#if isExpired}
        <div role="alert" class="p-2.5 rounded-lg bg-red-500/10 border border-red-500/20 text-xs text-red-400 flex items-center justify-between gap-2">
          <span>Plan expired. Inventory is valid for 15 min — scan again to refresh.</span>
          <div class="flex gap-1.5">
            <Button id="large-files-expiry-action" variant="ghost" size="sm" onclick={() => { plan = null; void scanFiles(); }}>Scan again</Button>
            <Button variant="ghost" size="sm" onclick={() => (plan = null)} class="text-red-400 hover:text-red-300">Dismiss</Button>
          </div>
        </div>
      {:else}
        <p class="text-[11px] text-muted-foreground">
          One-shot, expires at {new Date(plan.expires_at * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })} ({formatCountdown(remainingSecs)}). Zenith revalidates file identity and scope before each move.
        </p>
      {/if}
    </Card>
  {/if}

  <div class="flex flex-wrap items-center justify-between gap-3">
    <div>
      <h2 class="text-sm font-semibold">Results</h2>
      <p class="text-[11px] text-muted-foreground mt-0.5">
        {#if scanResult}
          {items.length} files · {entriesScanned.toLocaleString()} entries inspected
          {scanResult.cancelled ? ' · scan cancelled' : ''}
        {:else}
          Run a scan to inspect large files in approved personal folders.
        {/if}
      </p>
    </div>

    {#if items.length > 0}
      <div class="flex items-center gap-1.5">
        <Button variant="ghost" size="sm" onclick={selectAll} disabled={isScanning || isExecuting}>
          <CheckSquare size={13} />
          Select all
        </Button>
        <Button variant="ghost" size="sm" onclick={deselectAll} disabled={isScanning || isExecuting}>
          <Square size={13} />
          Clear
        </Button>
        <Button
          variant="primary"
          size="md"
          onclick={reviewTrash}
          disabled={isScanning || selectedIds.length === 0 || isPreparing || isExecuting}
          class="gap-1.5 ml-1"
        >
          <Trash2 size={14} />
          {isPreparing ? 'Reviewing…' : `Review ${formatBytes(selectedBytes)}`}
        </Button>
      </div>
    {/if}
  </div>

  {#if items.length > 0}
    <div class="space-y-2">
      {#each items as item (item.id)}
        <Card class="p-3.5">
          <div class="flex items-center gap-3">
            <input
              type="checkbox"
              checked={selectedIds.includes(item.id)}
              onchange={() => toggleItem(item.id)}
              disabled={isScanning || isExecuting}
              aria-label={`Select ${item.name}`}
              class="accent-emerald-500 shrink-0"
            />
            <div class="h-9 w-9 rounded-lg bg-secondary/60 border border-border/60 flex items-center justify-center shrink-0">
              <FileSearch size={16} class="text-muted-foreground" />
            </div>
            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-2">
                <span class="text-xs font-medium truncate">{item.name}</span>
                <span class="px-1.5 py-0.5 rounded text-[9px] bg-secondary text-muted-foreground border border-border/60">
                  {largeFileKindLabel(item.kind)}
                </span>
              </div>
              <div class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px] text-muted-foreground font-mono">
                <span class="truncate max-w-[520px]">{item.display_parent}</span>
                {#if item.modified_at}
                  <span>{formatTimeAgo(item.modified_at)}</span>
                {/if}
              </div>
            </div>
            <div class="text-right shrink-0">
              <div class="text-xs font-semibold font-mono">{formatBytes(item.allocated_size)}</div>
              {#if item.logical_size !== item.allocated_size}
                <div class="text-[9px] text-muted-foreground font-mono">{formatBytes(item.logical_size)} logical</div>
              {/if}
            </div>
            <Button
              variant="ghost"
              size="icon"
              onclick={() => tauriRevealInFinder(`${item.display_parent}/${item.name}`)}
              ariaLabel={`Reveal ${item.name} in Finder`}
              title="Reveal in Finder"
            >
              <FolderOpen size={14} />
            </Button>
          </div>
        </Card>
      {/each}
    </div>
  {:else if !isScanning}
    <Card class="py-14 text-center">
      <FileSearch size={26} class="mx-auto text-muted-foreground/50" />
      <p class="mt-3 text-xs text-muted-foreground">
        {scanResult ? 'No files matched this size threshold.' : 'No large-file scan yet.'}
      </p>
    </Card>
  {/if}
</div>
