<script lang="ts">
  import { onMount } from 'svelte';
  import type { CategoryResult, DiskVolume } from '../../lib/models/types';
  import { scanStore } from '../../lib/stores/scan.svelte';
  import {
    tauriGetDiskVolumes,
    tauriOpenDiskUtility,
    tauriRevealInFinder,
  } from '../../lib/utils/tauri';
  import { formatBytes } from '../../lib/utils/format';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import ProgressBar from '../../lib/components/ProgressBar.svelte';
  import { ExternalLink, FolderOpen, HardDrive, RefreshCw } from 'lucide-svelte';

  interface Props {
    onReviewCategory: (category: CategoryResult) => void;
  }

  let { onReviewCategory }: Props = $props();
  let volumes = $state<DiskVolume[]>([]);
  let isLoading = $state(false);
  let error = $state<string | null>(null);

  let primary = $derived(volumes.find((volume) => volume.is_primary) ?? volumes[0]);
  let opportunities = $derived.by(() =>
    (scanStore.lastScan?.categories ?? [])
      .filter((category) => category.total_bytes > 0)
      .toSorted((left, right) => right.total_bytes - left.total_bytes)
  );

  async function refresh() {
    if (isLoading) return;
    isLoading = true;
    error = null;
    try {
      volumes = await tauriGetDiskVolumes();
      if (!scanStore.lastScan) await scanStore.runScan();
    } catch (cause: any) {
      error = cause?.toString() || 'Could not inspect disks';
    } finally {
      isLoading = false;
    }
  }

  onMount(() => void refresh());
</script>

<div class="space-y-6">
  <div class="flex items-center justify-between pb-3 border-b border-border/60">
    <div class="flex items-center gap-3">
      <div class="h-9 w-9 rounded-lg bg-cyan-500/10 text-cyan-400 flex items-center justify-center">
        <HardDrive size={20} />
      </div>
      <div>
        <h2 class="text-base font-semibold tracking-tight">Disk Management</h2>
        <p class="text-xs text-muted-foreground mt-0.5">Volumes, free space, and cleanup opportunities</p>
      </div>
    </div>
    <div class="flex items-center gap-2">
      <Button variant="outline" size="sm" class="gap-1.5" onclick={() => tauriOpenDiskUtility()}>
        <ExternalLink size={13} />
        Disk Utility
      </Button>
      <Button variant="outline" size="sm" class="gap-1.5" disabled={isLoading} onclick={refresh}>
        <RefreshCw size={13} class={isLoading ? 'animate-gentle-spin' : ''} />
        Refresh
      </Button>
    </div>
  </div>

  {#if error}
    <div class="rounded-xl border border-destructive/20 bg-destructive/5 p-4 text-xs text-destructive">{error}</div>
  {:else if primary}
    <Card class="p-5">
      <div class="flex items-start justify-between gap-6">
        <div>
          <p class="text-caption uppercase tracking-wider text-muted-foreground">Primary storage</p>
          <p class="mt-1 text-sm font-semibold">{primary.name || 'Macintosh HD'}</p>
          <p class="mt-0.5 text-caption font-mono text-muted-foreground">{primary.mount_point} · {primary.file_system} · {primary.disk_type}</p>
        </div>
        <div class="text-right">
          <p class="font-mono text-2xl font-semibold">{formatBytes(primary.available_bytes)}</p>
          <p class="text-caption text-muted-foreground">available of {formatBytes(primary.total_bytes)}</p>
        </div>
      </div>
      <div class="mt-5 space-y-2">
        <ProgressBar value={primary.percent_used ?? 0} height="h-2" />
        <div class="flex justify-between text-caption font-mono text-muted-foreground">
          <span>{formatBytes(primary.used_bytes)} used</span>
          <span>{primary.percent_used != null ? `${primary.percent_used.toFixed(1)}%` : '—'}</span>
        </div>
      </div>
    </Card>
  {:else if isLoading}
    <div class="py-16 text-center text-xs text-muted-foreground">Reading mounted volumes…</div>
  {/if}

  {#if volumes.length > 0}
    <section class="space-y-3">
      <h3 class="text-sm font-semibold">Mounted Volumes</h3>
      <div class="grid grid-cols-2 gap-3">
        {#each volumes as volume (volume.mount_point)}
          <Card class="p-4">
            <div class="flex items-start justify-between gap-3">
              <div class="min-w-0">
                <div class="flex items-center gap-2">
                  <p class="truncate text-xs font-medium">{volume.name || volume.mount_point}</p>
                  {#if volume.is_primary}<span class="rounded-full border border-success/25 bg-success/10 px-1.5 py-0.5 text-micro text-success">Primary</span>{/if}
                  {#if volume.is_removable}<span class="rounded-full border border-border px-1.5 py-0.5 text-micro text-muted-foreground">External</span>{/if}
                </div>
                <p class="mt-1 truncate text-micro font-mono text-muted-foreground">{volume.mount_point}</p>
              </div>
              <Button variant="ghost" size="icon" class="h-7 w-7" ariaLabel="Reveal volume in Finder" title="Reveal in Finder" onclick={() => tauriRevealInFinder(volume.mount_point)}>
                <FolderOpen size={13} />
              </Button>
            </div>
            <div class="mt-4 space-y-1.5">
              <div class="flex justify-between text-caption">
                <span class="font-mono">{formatBytes(volume.used_bytes)} / {formatBytes(volume.total_bytes)}</span>
                <span class="text-muted-foreground">{volume.percent_used != null ? `${volume.percent_used.toFixed(1)}%` : '—'}</span>
              </div>
              <ProgressBar value={volume.percent_used ?? 0} height="h-1.5" />
            </div>
          </Card>
        {/each}
      </div>
    </section>
  {/if}

  <section class="space-y-3">
    <div class="flex items-end justify-between">
      <div>
        <h3 class="text-sm font-semibold">Cleanup Opportunities</h3>
        <p class="mt-0.5 text-caption text-muted-foreground">Largest detected categories first</p>
      </div>
      <span class="font-mono text-xs text-muted-foreground">{formatBytes(scanStore.lastScan?.total_bytes ?? 0)} detected</span>
    </div>
    <Card class="divide-y divide-border/60 overflow-hidden">
      {#each opportunities as category (category.category)}
        <button type="button" onclick={() => onReviewCategory(category)} class="w-full flex items-center justify-between px-4 py-3 text-left hover:bg-secondary/40 transition-colors">
          <div>
            <p class="text-xs font-medium">{category.display_name}</p>
            <p class="mt-0.5 text-caption text-muted-foreground">{formatBytes(category.safe_bytes)} safe · {formatBytes(category.rebuild_bytes)} rebuild</p>
          </div>
          <span class="font-mono text-xs font-semibold">{formatBytes(category.total_bytes)}</span>
        </button>
      {:else}
        <div class="px-4 py-8 text-center text-xs text-muted-foreground">Run a storage scan to find cleanup opportunities.</div>
      {/each}
    </Card>
  </section>
</div>
