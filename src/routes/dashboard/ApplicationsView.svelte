<script lang="ts">
  import { onMount } from 'svelte';
  import type {
    AppRelatedConfidence,
    AppUninstallInspection,
    InstalledApp,
    TrashPlanPreview,
    TrashResult,
  } from '../../lib/models/types';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import { formatBytes, formatCountdown, formatTimeAgo, ttlRemaining } from '../../lib/utils/format';
  import {
    defaultRelatedIds,
    selectedAppTrashBytes,
  } from '../../lib/utils/storageManagement';
  import {
    tauriExecuteTrashPlan,
    tauriGetInstalledApps,
    tauriInspectAppUninstall,
    tauriPrepareAppUninstall,
    tauriRevealInFinder,
  } from '../../lib/utils/tauri';
  import {
    AlertCircle,
    AppWindow,
    ArrowLeft,
    CheckCircle2,
    FolderOpen,
    RefreshCw,
    Search,
    ShieldAlert,
    ShieldCheck,
    Trash2,
  } from 'lucide-svelte';

  interface Props {
    onBack: () => void;
  }

  let { onBack }: Props = $props();

  let apps = $state<InstalledApp[]>([]);
  let query = $state('');
  let isLoading = $state(false);
  let isInspecting = $state(false);
  let isPreparing = $state(false);
  let isExecuting = $state(false);
  let inspection = $state<AppUninstallInspection | null>(null);
  let selectedRelatedIds = $state<string[]>([]);
  let plan = $state<TrashPlanPreview | null>(null);
  let trashResult = $state<TrashResult | null>(null);
  let error = $state<string | null>(null);

  let now = $state(Date.now());
  let remainingSecs = $derived(plan ? ttlRemaining(plan.expires_at, now) : 0);
  let isExpiringSoon = $derived(remainingSecs > 0 && remainingSecs <= 60);
  let isExpired = $derived(plan ? remainingSecs === 0 : false);

  let filteredApps = $derived(
    apps.filter((app) => {
      const needle = query.trim().toLowerCase();
      if (!needle) return true;
      return (
        app.name.toLowerCase().includes(needle) ||
        app.bundle_id?.toLowerCase().includes(needle) ||
        app.display_path.toLowerCase().includes(needle)
      );
    })
  );

  let selectedBytes = $derived(
    inspection ? selectedAppTrashBytes(inspection, selectedRelatedIds) : 0
  );

  function confidenceLabel(confidence: AppRelatedConfidence): string {
    switch (confidence) {
      case 'high':
        return 'High confidence';
      case 'medium':
        return 'Review';
      case 'shared':
        return 'Shared';
    }
  }

  function confidenceClass(confidence: AppRelatedConfidence): string {
    switch (confidence) {
      case 'high':
        return 'bg-emerald-500/10 text-emerald-500 border-emerald-500/20';
      case 'medium':
        return 'bg-amber-500/10 text-amber-500 border-amber-500/20';
      case 'shared':
        return 'bg-rose-500/10 text-rose-400 border-rose-500/20';
    }
  }

  function relatedKindLabel(kind: string): string {
    return kind
      .split('_')
      .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
      .join(' ');
  }

  async function loadApps() {
    isLoading = true;
    error = null;
    inspection = null;
    plan = null;
    trashResult = null;
    try {
      apps = await tauriGetInstalledApps();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      isLoading = false;
    }
  }

  async function inspectApp(app: InstalledApp) {
    if (app.is_running || app.is_system_protected) return;
    isInspecting = true;
    error = null;
    plan = null;
    trashResult = null;
    try {
      const result = await tauriInspectAppUninstall(app.id);
      inspection = result;
      selectedRelatedIds = defaultRelatedIds(result);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      isInspecting = false;
    }
  }

  function toggleRelated(id: string) {
    selectedRelatedIds = selectedRelatedIds.includes(id)
      ? selectedRelatedIds.filter((candidate) => candidate !== id)
      : [...selectedRelatedIds, id];
    plan = null;
    trashResult = null;
  }

  async function reviewUninstall() {
    if (!inspection) return;
    isPreparing = true;
    error = null;
    trashResult = null;
    try {
      plan = await tauriPrepareAppUninstall(
        inspection.inspection_id,
        selectedRelatedIds
      );
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      isPreparing = false;
    }
  }

  async function executeUninstall() {
    if (!plan || !inspection) return;
    isExecuting = true;
    error = null;
    const appId = inspection.app.id;
    try {
      const result = await tauriExecuteTrashPlan(plan.id);
      trashResult = result;
      if (result.items.some((item) => item.item_id === appId && item.success)) {
        apps = apps.filter((app) => app.id !== appId);
        inspection = null;
        selectedRelatedIds = [];
      }
      plan = null;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      isExecuting = false;
    }
  }

  onMount(() => {
    void loadApps();
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });
</script>

<div class="space-y-5">
  <div class="flex items-start justify-between gap-4">
    <div class="flex items-start gap-3">
      <Button variant="ghost" size="icon" onclick={onBack} ariaLabel="Back to Storage">
        <ArrowLeft size={16} />
      </Button>
      <div>
        <h1 class="text-xl font-semibold tracking-tight">Applications</h1>
        <p class="mt-1 text-xs text-muted-foreground">
          Review an app bundle and only the related Library data Zenith can identify with constrained rules.
        </p>
      </div>
    </div>
    <div class="flex items-center gap-1.5 text-[11px] text-muted-foreground">
      <ShieldCheck size={14} class="text-emerald-500" />
      <span>Moves to Trash, never permanent delete</span>
    </div>
  </div>

  {#if error}
    <div class="p-3.5 rounded-xl bg-destructive/15 border border-destructive/30 text-destructive flex items-center gap-2.5 text-xs">
      <AlertCircle size={16} class="shrink-0" />
      <span>{error}</span>
    </div>
  {/if}

  {#if trashResult}
    <Card class={`p-4 ${trashResult.failed_count + trashResult.skipped_count > 0 ? 'border-amber-500/30 bg-amber-500/5' : 'border-emerald-500/30 bg-emerald-500/5'}`}>
      <div class="flex items-center justify-between gap-3 text-xs">
        <span class={`font-medium flex items-center gap-2 ${trashResult.failed_count + trashResult.skipped_count > 0 ? 'text-amber-500' : 'text-emerald-500'}`}>
          {#if trashResult.failed_count + trashResult.skipped_count > 0}
            <AlertCircle size={15} />
          {:else}
            <CheckCircle2 size={15} />
          {/if}
          Moved {trashResult.moved_count} reviewed item{trashResult.moved_count === 1 ? '' : 's'} to Trash
          {#if trashResult.failed_count + trashResult.skipped_count > 0}
            · {trashResult.failed_count + trashResult.skipped_count} not moved
          {/if}
        </span>
        <span class="font-mono text-muted-foreground">{formatBytes(trashResult.moved_allocated_size)}</span>
      </div>
    </Card>
  {/if}

  <div class="grid grid-cols-1 xl:grid-cols-[minmax(300px,0.85fr)_minmax(440px,1.35fr)] gap-4 items-start">
    <Card class="p-4 space-y-3 xl:sticky xl:top-0">
      <div class="flex items-center justify-between gap-2">
        <div>
          <h2 class="text-sm font-semibold">Installed apps</h2>
          <p class="text-[10px] text-muted-foreground mt-0.5">/Applications and ~/Applications</p>
        </div>
        <Button variant="ghost" size="icon" onclick={loadApps} disabled={isLoading} ariaLabel="Refresh applications">
          <RefreshCw size={14} class={isLoading ? 'animate-gentle-spin' : ''} />
        </Button>
      </div>

      <div class="relative">
        <Search size={13} class="absolute left-2.5 top-2.5 text-muted-foreground" />
        <input
          type="search"
          bind:value={query}
          placeholder="Search applications"
          class="w-full h-8 rounded-lg border border-border bg-background pl-8 pr-3 text-xs outline-none focus:border-ring"
        />
      </div>

      <div class="space-y-1 max-h-[calc(100vh-245px)] overflow-y-auto pr-1">
        {#if isLoading}
          <div class="py-10 text-center text-xs text-muted-foreground">Loading applications…</div>
        {:else if filteredApps.length === 0}
          <div class="py-10 text-center text-xs text-muted-foreground">No applications found.</div>
        {:else}
          {#each filteredApps as app (app.id)}
            <button
              type="button"
              onclick={() => inspectApp(app)}
              disabled={isInspecting || app.is_running || app.is_system_protected}
              class="w-full p-2.5 rounded-lg border text-left transition-colors disabled:opacity-50 disabled:cursor-not-allowed {inspection?.app.id === app.id
                ? 'border-primary/40 bg-primary/5'
                : 'border-transparent hover:border-border hover:bg-secondary/40'}"
            >
              <div class="flex items-center gap-2.5">
                <div class="h-8 w-8 rounded-lg bg-secondary/70 border border-border/60 flex items-center justify-center shrink-0">
                  <AppWindow size={15} class="text-muted-foreground" />
                </div>
                <div class="min-w-0 flex-1">
                  <div class="flex items-center gap-2">
                    <span class="text-xs font-medium truncate">{app.name}</span>
                    {#if app.is_running}
                      <span class="px-1.5 py-0.5 rounded text-[9px] bg-amber-500/10 text-amber-500 border border-amber-500/20">Running</span>
                    {/if}
                  </div>
                  <div class="text-[10px] text-muted-foreground font-mono truncate mt-0.5">
                    {formatBytes(app.allocated_size)}{app.version ? ` · ${app.version}` : ''}
                  </div>
                </div>
              </div>
            </button>
          {/each}
        {/if}
      </div>
    </Card>

    <div class="space-y-4">
      {#if inspection}
        <Card class="p-5 space-y-4">
          <div class="flex flex-col sm:flex-row sm:items-start justify-between gap-3">
            <div>
              <div class="flex flex-wrap items-center gap-2">
                <h2 class="text-base font-semibold">{inspection.app.name}</h2>
                {#if inspection.app.version}
                  <span class="px-1.5 py-0.5 rounded text-[9px] border border-border bg-secondary/50 text-muted-foreground">v{inspection.app.version}</span>
                {/if}
              </div>
              <p class="mt-1 text-[10px] text-muted-foreground font-mono break-all">
                {inspection.app.bundle_id ?? 'No bundle identifier'}
              </p>
              <div class="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[10px] text-muted-foreground">
                <span>{formatBytes(inspection.app.allocated_size)} app bundle</span>
                {#if inspection.app.modified_at}
                  <span>Modified {formatTimeAgo(inspection.app.modified_at)}</span>
                {/if}
              </div>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onclick={() => tauriRevealInFinder(inspection!.app.display_path)}
              class="gap-1.5"
            >
              <FolderOpen size={13} />
              Finder
            </Button>
          </div>

          {#if inspection.warnings.length > 0 || inspection.incomplete}
            <div class="rounded-lg border border-amber-500/25 bg-amber-500/5 p-3 space-y-1.5 text-[11px] text-amber-500">
              <div class="flex items-center gap-1.5 font-medium">
                <ShieldAlert size={14} />
                Inspection warnings
              </div>
              {#each inspection.warnings as warning}
                <p>{warning}</p>
              {/each}
              {#if inspection.incomplete && inspection.warnings.length === 0}
                <p>Some protected or unreadable locations could not be inspected.</p>
              {/if}
            </div>
          {/if}

          <div class="pt-3 border-t border-border/60 space-y-2">
            <div class="flex items-center justify-between gap-3">
              <div>
                <h3 class="text-xs font-semibold">Related Library data</h3>
                <p class="text-[10px] text-muted-foreground mt-0.5">
                  Only high-confidence exact bundle matches are selected by default.
                </p>
              </div>
              <span class="text-[10px] text-muted-foreground font-mono">
                {selectedRelatedIds.length} selected
              </span>
            </div>

            {#if inspection.related_items.length === 0}
              <div class="py-6 text-center text-xs text-muted-foreground border border-dashed border-border rounded-lg">
                No related data matched the constrained resolver.
              </div>
            {:else}
              <div class="space-y-2">
                {#each inspection.related_items as item (item.id)}
                  <label class="block p-3 rounded-lg border border-border/70 bg-secondary/15 cursor-pointer hover:bg-secondary/30">
                    <div class="flex items-start gap-3">
                      <input
                        type="checkbox"
                        checked={selectedRelatedIds.includes(item.id)}
                        onchange={() => toggleRelated(item.id)}
                        disabled={isExecuting}
                        class="mt-0.5 accent-emerald-500"
                      />
                      <div class="min-w-0 flex-1">
                        <div class="flex flex-wrap items-center gap-2">
                          <span class="text-xs font-medium truncate">{item.name}</span>
                          <span class={`px-1.5 py-0.5 rounded text-[9px] border ${confidenceClass(item.confidence)}`}>
                            {confidenceLabel(item.confidence)}
                          </span>
                          <span class="px-1.5 py-0.5 rounded text-[9px] border border-border bg-secondary/50 text-muted-foreground">
                            {relatedKindLabel(item.kind)}
                          </span>
                        </div>
                        <p class="text-[10px] text-muted-foreground font-mono break-all mt-1">{item.display_path}</p>
                        <p class="text-[10px] text-muted-foreground mt-1">{item.evidence}</p>
                      </div>
                      <span class="text-[10px] font-mono shrink-0">{formatBytes(item.allocated_size)}</span>
                    </div>
                  </label>
                {/each}
              </div>
            {/if}
          </div>
        </Card>

        {#if plan}
          <Card class={`p-5 space-y-3 ${isExpired ? 'border-red-500/40 bg-red-500/5' : isExpiringSoon ? 'border-amber-500/50 bg-amber-500/10' : 'border-amber-500/30 bg-amber-500/5'}`}>
            <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
              <div>
                <div class="text-sm font-semibold flex items-center gap-2">
                  Uninstall review ready
                  <span class={`text-[10px] px-1.5 py-0.5 rounded font-mono border ${isExpired ? 'bg-red-500/15 text-red-400 border-red-500/30' : isExpiringSoon ? 'bg-amber-500/15 text-amber-500 border-amber-500/30' : 'bg-secondary text-muted-foreground border-border'}`}>
                    {formatCountdown(remainingSecs)}
                  </span>
                </div>
                <p class="text-xs text-muted-foreground mt-1">
                  App bundle plus reviewed data: {plan.item_count} items · {formatBytes(plan.allocated_size)}
                </p>
              </div>
              <div class="flex items-center gap-2">
                <Button variant="ghost" size="sm" onclick={() => (plan = null)}>Cancel</Button>
                <Button variant="destructive" size="md" onclick={executeUninstall} disabled={isExecuting || isExpired} class="gap-1.5" title={isExpired ? 'Plan expired — review again' : ''}>
                  <Trash2 size={14} />
                  {isExecuting ? 'Moving…' : isExpired ? 'Expired' : 'Move App to Trash'}
                </Button>
              </div>
            </div>
            {#if isExpired}
              <div class="p-2.5 rounded-lg bg-red-500/10 border border-red-500/20 text-xs text-red-400 flex items-center justify-between gap-2">
                <span>Plan expired — review again to refresh the 5 min window.</span>
                <div class="flex gap-1.5">
                  <Button variant="ghost" size="sm" onclick={() => { plan = null; void reviewUninstall(); }} autofocus>Review again</Button>
                  <Button variant="ghost" size="sm" onclick={() => (plan = null)} class="text-red-400 hover:text-red-300">Dismiss</Button>
                </div>
              </div>
            {:else}
              <p class="text-[11px] text-muted-foreground">
                One-shot, expires at {new Date(plan.expires_at * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })} ({formatCountdown(remainingSecs)}). Zenith rechecks the app and each selected Library item immediately before moving them to Trash.
              </p>
            {/if}
          </Card>
        {:else}
          <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-3 px-1">
            <div>
              <div class="text-xs font-medium">Reviewed Trash size</div>
              <div class="text-lg font-semibold font-mono mt-0.5">{formatBytes(selectedBytes)}</div>
            </div>
            <Button
              variant="primary"
              size="md"
              onclick={reviewUninstall}
              disabled={isPreparing || isExecuting}
              class="gap-1.5 min-w-[150px]"
            >
              <ShieldCheck size={14} />
              {isPreparing ? 'Preparing…' : 'Review Uninstall'}
            </Button>
          </div>
        {/if}
      {:else}
        <Card class="py-20 text-center">
          <AppWindow size={28} class="mx-auto text-muted-foreground/50" />
          <p class="mt-3 text-sm font-medium">Choose an application</p>
          <p class="mt-1 text-xs text-muted-foreground max-w-md mx-auto px-8">
            Running apps are intentionally blocked. Zenith first builds a backend-owned inventory, then resolves related data with exact bundle or exact app-name matches only.
          </p>
        </Card>
      {/if}
    </div>
  </div>
</div>
