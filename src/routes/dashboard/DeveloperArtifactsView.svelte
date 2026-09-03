<script lang="ts">
  import { tick } from 'svelte';
  import type {
    DeveloperArtifact,
    DeveloperArtifactStatus,
    DeveloperArtifactScanEvent,
    DeveloperArtifactScanResult,
    DeveloperWorkspace,
    TrashPlanPreview,
    TrashResult,
  } from '../../lib/models/types';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import { formatBytes, formatCountdown, formatTimeAgo, ttlRemaining } from '../../lib/utils/format';
  import {
    tauriCancelDeveloperArtifactScan,
    tauriExecuteTrashPlan,
    tauriPickDeveloperWorkspace,
    tauriPrepareDeveloperArtifactCleanup,
    tauriRegisterDeveloperHomeWorkspace,
    tauriRevealInFinder,
    tauriStartDeveloperArtifactScan,
  } from '../../lib/utils/tauri';
  import {
    AlertCircle,
    ArrowLeft,
    CheckSquare,
    FolderOpen,
    HardDrive,
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

  type SortKey = 'size' | 'activity' | 'project' | 'type';

  let workspaces = $state<DeveloperWorkspace[]>([]);
  let selectedWorkspaceIds = $state<string[]>([]);
  let items = $state<DeveloperArtifact[]>([]);
  let scanResult = $state<DeveloperArtifactScanResult | null>(null);
  let plan = $state<TrashPlanPreview | null>(null);
  let trashResult = $state<TrashResult | null>(null);
  let selectedIds = $state<string[]>([]);
  let sortBy = $state<SortKey>('size');
  let isScanning = $state(false);
  let isPreparing = $state(false);
  let isExecuting = $state(false);
  let activeScanId = $state<string | null>(null);
  let activeWorkspace = $state('');
  let discoveredCount = $state(0);
  let measuredCount = $state(0);
  let skippedEntries = $state(0);
  let error = $state<string | null>(null);
  let now = $state(Date.now());
  let expiryActionFocused = $state(false);
  let partialCleanupConfirmed = $state(false);
  let selectedIdSet = $derived(new Set(selectedIds));
  let selectedItems = $derived(items.filter((item) => selectedIdSet.has(item.id)));
  let selectedBytes = $derived(selectedItems.reduce((total, item) => total + item.allocated_bytes, 0));
  let hasMeasurementIncompleteSelected = $derived(
    selectedItems.some((item) => item.status === 'measurement_incomplete')
  );
  let sortedItems = $derived(
    [...items].sort((left, right) => {
      if (sortBy === 'activity') {
        return (right.newest_mtime ?? 0) - (left.newest_mtime ?? 0);
      }
      if (sortBy === 'project') {
        return left.project_name.localeCompare(right.project_name);
      }
      if (sortBy === 'type') {
        return kindLabel(left).localeCompare(kindLabel(right));
      }
      return right.allocated_bytes - left.allocated_bytes;
    })
  );
  let remainingSecs = $derived(plan ? ttlRemaining(plan.expires_at, now) : 0);
  let isExpired = $derived(plan ? remainingSecs === 0 : false);

  $effect(() => {
    if (!plan) return;
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  $effect(() => {
    if (!isExpired || expiryActionFocused) return;
    expiryActionFocused = true;
    void tick().then(() => document.getElementById('developer-artifact-expiry-action')?.focus());
  });

  function kindLabel(item: DeveloperArtifact): string {
    const labels: Record<DeveloperArtifact['kind'], string> = {
      cargo_target: 'Rust build output',
      node_modules: 'Node dependencies',
      python_venv: 'Python environment',
      go_module_cache: 'Go module cache',
      maven_target: 'Maven target',
      gradle_build: 'Gradle build',
      gradle_cache: 'Gradle cache',
      composer_vendor: 'Composer vendor',
      ruby_bundle: 'Ruby bundle',
      dotnet_bin: '.NET bin',
      dotnet_obj: '.NET obj',
      c_make_build: 'CMake build',
      swift_build: 'Swift build',
      flutter_tooling: 'Flutter tooling',
      elixir_build: 'Elixir build',
      elixir_deps: 'Elixir dependencies',
      terraform_cache: 'Terraform cache',
    };
    return labels[item.kind];
  }

  function cleanupScopeLabel(item: DeveloperArtifact): string {
    const labels: Record<DeveloperArtifact['kind'], string> = {
      cargo_target: 'target/',
      node_modules: 'node_modules/',
      python_venv: item.path.endsWith('/.venv') ? '.venv/' : 'venv/',
      go_module_cache: 'pkg/mod/',
      maven_target: 'target/',
      gradle_build: 'build/',
      gradle_cache: '.gradle/',
      composer_vendor: 'vendor/',
      ruby_bundle: 'vendor/bundle/',
      dotnet_bin: 'bin/',
      dotnet_obj: 'obj/',
      c_make_build: 'build/',
      swift_build: '.build/',
      flutter_tooling: '.dart_tool/',
      elixir_build: '_build/',
      elixir_deps: 'deps/',
      terraform_cache: '.terraform/',
    };
    return labels[item.kind];
  }

  function ecosystemLabel(item: DeveloperArtifact): string {
    return item.ecosystem === 'dotnet'
      ? '.NET'
      : item.ecosystem.charAt(0).toUpperCase() + item.ecosystem.slice(1);
  }

  function resetReview() {
    plan = null;
    trashResult = null;
    expiryActionFocused = false;
    partialCleanupConfirmed = false;
  }

  function canManuallyClean(item: DeveloperArtifact): boolean {
    return item.status === 'complete' || item.status === 'measurement_incomplete';
  }

  function statusLabel(status: DeveloperArtifactStatus): string {
    switch (status) {
      case 'measurement_incomplete':
        return 'Review with warning';
      case 'safety_blocked':
        return 'Blocked · safety check';
      case 'scan_cancelled':
        return 'Blocked · scan incomplete';
      default:
        return '';
    }
  }

  async function addWorkspace() {
    error = null;
    try {
      const workspace = await tauriPickDeveloperWorkspace();
      if (!workspace || workspaces.some((candidate) => candidate.id === workspace.id)) return;
      workspaces = [...workspaces, workspace];
      selectedWorkspaceIds = [...selectedWorkspaceIds, workspace.id];
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  function removeWorkspace(id: string) {
    workspaces = workspaces.filter((workspace) => workspace.id !== id);
    selectedWorkspaceIds = selectedWorkspaceIds.filter((workspaceId) => workspaceId !== id);
    if (selectedWorkspaceIds.length === 0) {
      items = [];
      scanResult = null;
      selectedIds = [];
    }
  }

  function handleScanEvent(event: DeveloperArtifactScanEvent) {
    switch (event.type) {
      case 'started':
        activeScanId = event.scan_id;
        discoveredCount = 0;
        measuredCount = 0;
        skippedEntries = 0;
        break;
      case 'workspace_started':
        activeWorkspace = event.workspace.display_path;
        break;
      case 'artifact_found':
        if (!items.some((item) => item.id === event.artifact.id)) {
          items = [...items, event.artifact];
        }
        break;
      case 'progress':
        activeWorkspace = workspaces.find((workspace) => workspace.id === event.workspace_id)?.display_path ?? activeWorkspace;
        discoveredCount = event.discovered_count;
        measuredCount = event.measured_count;
        skippedEntries = event.skipped_entries;
        break;
      case 'finished':
        scanResult = event.result;
        items = event.result.items;
        discoveredCount = event.result.discovered_count;
        measuredCount = event.result.measured_count;
        skippedEntries = event.result.skipped_entries;
        break;
      case 'cancelled':
        activeScanId = event.scan_id;
        break;
      case 'project_discovered':
      case 'artifact_measurement_started':
      case 'workspace_finished':
        break;
    }
  }

  async function runScan(workspaceIds: string[]) {
    if (workspaceIds.length === 0) {
      error = 'Add at least one workspace before scanning.';
      return;
    }
    isScanning = true;
    error = null;
    resetReview();
    scanResult = null;
    items = [];
    selectedIds = [];
    activeScanId = null;
    activeWorkspace = '';
    discoveredCount = 0;
    measuredCount = 0;
    skippedEntries = 0;
    try {
      const result = await tauriStartDeveloperArtifactScan(workspaceIds, handleScanEvent);
      scanResult = result;
      items = result.items;
      discoveredCount = result.discovered_count;
      measuredCount = result.measured_count;
      skippedEntries = result.skipped_entries;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      isScanning = false;
      activeScanId = null;
      activeWorkspace = '';
    }
  }

  async function scanArtifacts() {
    await runScan(selectedWorkspaceIds);
  }

  async function scanThisComputer() {
    error = null;
    try {
      const workspace = await tauriRegisterDeveloperHomeWorkspace();
      workspaces = [workspace];
      selectedWorkspaceIds = [workspace.id];
      await runScan([workspace.id]);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  async function cancelScan() {
    if (!activeScanId) return;
    try {
      await tauriCancelDeveloperArtifactScan(activeScanId);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  function toggleItem(item: DeveloperArtifact) {
    if (!canManuallyClean(item) || isScanning || isExecuting) return;
    selectedIds = selectedIdSet.has(item.id)
      ? selectedIds.filter((id) => id !== item.id)
      : [...selectedIds, item.id];
    resetReview();
  }

  function selectAll() {
    selectedIds = items.filter((item) => item.status === 'complete').map((item) => item.id);
    resetReview();
  }

  function deselectAll() {
    selectedIds = [];
    resetReview();
  }

  async function reviewCleanup() {
    if (!scanResult || selectedIds.length === 0) return;
    isPreparing = true;
    error = null;
    try {
      plan = await tauriPrepareDeveloperArtifactCleanup(scanResult.scan_id, selectedIds);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      isPreparing = false;
    }
  }

  async function executeCleanup() {
    if (!plan) return;
    if (hasMeasurementIncompleteSelected && !partialCleanupConfirmed) {
      error = 'Confirm the partial-measurement warning before moving these artifacts to Trash.';
      return;
    }
    isExecuting = true;
    error = null;
    try {
      const result = await tauriExecuteTrashPlan(plan.id);
      trashResult = result;
      const moved = new Set(result.items.filter((item) => item.success).map((item) => item.item_id));
      items = items.filter((item) => !moved.has(item.id));
      selectedIds = selectedIds.filter((id) => !moved.has(id));
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
        <h1 class="text-xl font-semibold tracking-tight">Developer Artifacts</h1>
        <div class="flex shrink-0 items-center gap-1.5 text-meta text-muted-foreground">
          <ShieldCheck size={14} class="text-success" />
          <span>Review only · nothing selected by default</span>
        </div>
      </div>
      <p class="mt-1 text-xs text-muted-foreground">
        Inspect rebuildable project environments across common ecosystems. Project source, manifests, lockfiles, and project roots are never cleanup targets.
      </p>
    </div>
  </div>

  <Card class="p-5 space-y-4">
    <div class="flex flex-wrap items-start justify-between gap-3">
      <div>
        <div class="text-xs font-medium">Scan scope</div>
        <p class="mt-1 text-meta text-muted-foreground">Scan your user-owned files in one pass. System, credential, media, and installed-application paths are bypassed.</p>
      </div>
      <div class="flex items-center gap-2">
        {#if isScanning && activeScanId}
          <Button variant="outline" size="md" onclick={cancelScan} class="gap-1.5">
            <X size={14} />
            Cancel
          </Button>
        {/if}
        <Button variant="primary" size="md" onclick={scanThisComputer} disabled={isScanning} class="gap-1.5">
          {#if isScanning}<DeletingDots size="sm" />{:else}<HardDrive size={14} />{/if}
          {isScanning ? 'Scanning this computer…' : 'Scan this computer'}
        </Button>
        <Button variant="outline" size="md" onclick={addWorkspace} disabled={isScanning} class="gap-1.5">
          <FolderOpen size={14} />
          Add folder
        </Button>
        <Button variant="outline" size="md" onclick={scanArtifacts} disabled={isScanning || selectedWorkspaceIds.length === 0} class="gap-1.5">
          {#if isScanning}<DeletingDots size="sm" />{:else}<RefreshCw size={14} />{/if}
          Scan selected
        </Button>
      </div>
    </div>

    {#if workspaces.length === 0}
      <div class="rounded-lg border border-dashed border-border/80 p-4 text-xs text-muted-foreground">
        <span class="font-medium text-foreground">Scan this computer</span> searches your user-owned files automatically. Add a folder only when you want a narrower scan.
      </div>
    {:else}
      <div class="grid gap-2 sm:grid-cols-2">
        {#each workspaces as workspace (workspace.id)}
          <div class="flex items-center gap-2 rounded-lg border border-border/70 bg-secondary/20 px-3 py-2">
            <input
              type="checkbox"
              checked={selectedWorkspaceIds.includes(workspace.id)}
              onchange={() => {
                selectedWorkspaceIds = selectedWorkspaceIds.includes(workspace.id)
                  ? selectedWorkspaceIds.filter((id) => id !== workspace.id)
                  : [...selectedWorkspaceIds, workspace.id];
              }}
              disabled={isScanning}
              aria-label={`Scan ${workspace.name}`}
              class="accent-success"
            />
            <div class="min-w-0 flex-1">
              <div class="truncate text-xs font-medium">{workspace.name}</div>
              <div class="truncate font-mono text-caption text-muted-foreground">{workspace.display_path}</div>
            </div>
            <Button variant="ghost" size="icon" class="h-7 w-7" onclick={() => removeWorkspace(workspace.id)} disabled={isScanning} ariaLabel={`Remove ${workspace.name}`} title="Remove workspace">
              <X size={13} />
            </Button>
          </div>
        {/each}
      </div>
    {/if}

    {#if isScanning}
      <div class="flex flex-wrap items-center justify-between gap-2 border-t border-border/60 pt-3 text-meta text-muted-foreground">
        <span>Scanning {activeWorkspace || 'selected workspaces'}…</span>
        <span class="font-mono">{measuredCount} / {discoveredCount} measured · {skippedEntries} skipped</span>
      </div>
    {/if}
  </Card>

  {#if error}
    <div class="flex items-center gap-2.5 rounded-xl border border-destructive/30 bg-destructive/15 p-3.5 text-xs text-destructive">
      <AlertCircle size={16} class="shrink-0" />
      <span>{error}</span>
    </div>
  {/if}

  {#if trashResult}
    <Card class={`p-4 ${trashResult.failed_count + trashResult.skipped_count > 0 ? 'border-warning/30 bg-warning/5' : 'border-success/30 bg-success/5'}`}>
      <div class="flex items-center justify-between gap-3 text-xs">
        <span class={`font-medium ${trashResult.failed_count + trashResult.skipped_count > 0 ? 'text-warning' : 'text-success'}`}>
          Moved {trashResult.moved_count} artifact{trashResult.moved_count === 1 ? '' : 's'} to Trash
        </span>
        <span class="font-mono text-muted-foreground">{formatBytes(trashResult.moved_allocated_size)} · empty Trash to reclaim</span>
      </div>
    </Card>
  {/if}

  {#if plan}
    <Card class={`space-y-3 p-5 ${isExpired ? 'border-destructive/40 bg-destructive/5' : 'border-warning/30 bg-warning/5'}`}>
      <div class="flex flex-col justify-between gap-3 lg:flex-row lg:items-center">
        <div>
          <div class="flex items-center gap-2 text-sm font-semibold">
            Generated-folder review ready
            <span class="rounded border border-border bg-secondary px-1.5 py-0.5 font-mono text-caption text-muted-foreground">{formatCountdown(remainingSecs)}</span>
          </div>
          <p class="mt-1 text-xs text-muted-foreground">{plan.item_count} selected artifact{plan.item_count === 1 ? '' : 's'} · {formatBytes(plan.allocated_size)} allocated</p>
        </div>
        <div class="flex items-center gap-2">
          <Button variant="ghost" size="sm" onclick={() => { plan = null; partialCleanupConfirmed = false; }}>Cancel</Button>
          <Button variant="destructive" size="md" onclick={executeCleanup} disabled={isExecuting || isExpired || (hasMeasurementIncompleteSelected && !partialCleanupConfirmed)} class="gap-1.5">
            {#if isExecuting}<DeletingDots size="sm" />{:else}<Trash2 size={14} />{/if}
            {isExecuting ? 'Moving…' : isExpired ? 'Expired' : 'Move generated folders to Trash'}
          </Button>
        </div>
      </div>
      <div class="rounded-lg border border-border/70 bg-background/60 p-3">
        <p class="text-meta text-muted-foreground">Only the exact generated directories below will move. Project code and configuration stay in place.</p>
        <div class="mt-2 max-h-32 space-y-1.5 overflow-y-auto">
          {#each selectedItems as item (item.id)}
            <div class="flex items-center justify-between gap-3 text-caption">
              <span class="min-w-0 truncate font-mono" title={item.path}>{item.path}</span>
              <span class="shrink-0 text-muted-foreground">{formatBytes(item.allocated_bytes)}</span>
            </div>
          {/each}
        </div>
      </div>
      {#if isExpired}
        <div role="alert" class="flex items-center justify-between gap-2 rounded-lg border border-destructive/20 bg-destructive/10 p-2.5 text-xs text-destructive">
          <span>Plan expired. Scan again to refresh the inventory.</span>
          <Button id="developer-artifact-expiry-action" variant="ghost" size="sm" onclick={() => { plan = null; void scanArtifacts(); }}>Scan again</Button>
        </div>
      {:else}
        <p class="text-meta text-muted-foreground">One-shot plan. Zenith rechecks workspace identity, markers, exact artifact type, symlinks, and filesystem identity before each move.</p>
        {#if hasMeasurementIncompleteSelected}
          <div class="space-y-2 rounded-lg border border-warning/30 bg-warning/10 p-3 text-xs text-warning">
            <p class="font-medium">Some selected measurements are partial.</p>
            <p class="text-warning/90">The generated-folder scope and project evidence were verified, but one or more entries could not be measured. The displayed size and file count may be lower than the actual contents. Nested links are not followed.</p>
            <label class="flex items-start gap-2 text-warning">
              <input type="checkbox" bind:checked={partialCleanupConfirmed} class="mt-0.5 accent-warning" />
              <span>I understand the measurements may be partial and want to move the verified generated folder(s) to Trash.</span>
            </label>
          </div>
        {/if}
      {/if}
    </Card>
  {/if}

  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h2 class="text-sm font-semibold">Reviewable project artifacts</h2>
      <p class="mt-0.5 text-meta text-muted-foreground">
        {#if scanResult}
          {items.length} candidate{items.length === 1 ? '' : 's'} · {measuredCount} measured{scanResult.cancelled ? ' · scan cancelled' : ''}{scanResult.truncated ? ' · result cap reached' : ''}
        {:else}
          Rust, Node, Python, Java/Kotlin, PHP, Ruby, .NET, C/C++, Swift, Dart, Elixir, Terraform, and Go markers are supported.
        {/if}
      </p>
    </div>
    <div class="flex flex-wrap items-center gap-2">
      <label for="developer-artifact-sort" class="text-meta text-muted-foreground">Sort</label>
      <select id="developer-artifact-sort" bind:value={sortBy} class="h-8 rounded-lg border border-border bg-background px-2.5 text-xs">
        <option value="size">Largest first</option>
        <option value="activity">Most recently changed</option>
        <option value="project">Project name</option>
        <option value="type">Artifact type</option>
      </select>
      {#if items.length > 0}
        <Button variant="ghost" size="sm" onclick={selectAll} disabled={isScanning || isExecuting}><CheckSquare size={13} /> Select rebuildable</Button>
        <Button variant="ghost" size="sm" onclick={deselectAll} disabled={isScanning || isExecuting}><Square size={13} /> Clear</Button>
        <Button variant="primary" size="md" onclick={reviewCleanup} disabled={isScanning || isPreparing || isExecuting || selectedIds.length === 0} class="gap-1.5">
          {#if isPreparing}<DeletingDots size="sm" />{:else}<Trash2 size={14} />{/if}
          {isPreparing ? 'Reviewing…' : `Review ${formatBytes(selectedBytes)}`}
        </Button>
      {/if}
    </div>
  </div>

  {#if sortedItems.length === 0}
    <Card class="p-8 text-center text-xs text-muted-foreground">No recognized developer artifacts yet. Add a workspace and run a scan.</Card>
  {:else}
    <div class="space-y-2">
      {#each sortedItems as item (item.id)}
        <Card class={`p-4 ${item.status === 'complete' ? '' : item.status === 'measurement_incomplete' ? 'border-warning/40 bg-warning/5' : 'border-destructive/30 bg-destructive/5'}`}>
          <div class="flex items-start gap-3">
            <input
              type="checkbox"
              checked={selectedIdSet.has(item.id)}
              onchange={() => toggleItem(item)}
              disabled={!canManuallyClean(item) || isScanning || isExecuting}
              aria-label={`Select ${item.project_name} ${kindLabel(item)}${item.status === 'measurement_incomplete' ? ' with partial measurement warning' : ''}`}
              class="mt-1 accent-success"
            />
            <div class="min-w-0 flex-1 space-y-1.5">
              <div class="flex flex-wrap items-center gap-2">
                <span class="text-xs font-semibold">{item.project_name}</span>
                <span class="rounded bg-secondary px-1.5 py-0.5 text-caption text-muted-foreground">{ecosystemLabel(item)} · {kindLabel(item)}</span>
                {#if item.status !== 'complete'}
                  <span class={`rounded border px-1.5 py-0.5 text-caption ${item.status === 'measurement_incomplete' ? 'border-warning/30 bg-warning/10 text-warning' : 'border-destructive/30 bg-destructive/10 text-destructive'}`}>{statusLabel(item.status)}</span>
                {/if}
              </div>
              <div class="truncate font-mono text-caption text-muted-foreground" title={item.path}>{item.path}</div>
              <div class="flex flex-wrap gap-x-3 gap-y-1 text-meta text-muted-foreground">
                <span>{formatBytes(item.allocated_bytes)} allocated</span>
                <span>{item.file_count.toLocaleString()} files</span>
                <span>Last changed {formatTimeAgo(item.newest_mtime ?? undefined)}</span>
                {#if item.rebuild_hint}<span>↻ {item.rebuild_hint}</span>{/if}
              </div>
              <div class="flex flex-wrap items-center gap-2 text-caption text-muted-foreground">
                <span>Cleanup scope: <span class="font-mono text-foreground">{cleanupScopeLabel(item)}</span> only · source stays</span>
                <span>Evidence: {item.evidence.join(' · ')}</span>
                {#if item.incomplete_reason}<span class={item.status === 'measurement_incomplete' ? 'text-warning' : 'text-destructive'}>{item.incomplete_reason}</span>{/if}
              </div>
            </div>
            <Button variant="ghost" size="icon" class="h-7 w-7 shrink-0" onclick={() => tauriRevealInFinder(item.path)} ariaLabel={`Show ${item.path} in file manager`} title="Show in File Manager">
              <FolderOpen size={13} />
            </Button>
          </div>
        </Card>
      {/each}
    </div>
  {/if}
</div>
