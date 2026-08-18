<script lang="ts">
  import { onMount } from 'svelte';
  import { dockerStore } from '../../lib/stores/docker.svelte';
  import { formatBytes } from '../../lib/utils/format';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import {
    Container,
    RotateCw,
    Trash2,
    Layers,
    Server,
    HardDrive,
    AlertCircle,
  } from 'lucide-svelte';

  let status = $derived(dockerStore.status);
  let overview = $derived(status?.overview);
  let confirmVolumePrune = $state(false);

  onMount(() => {
    void dockerStore.refresh();
  });

  async function pruneVolumes() {
    confirmVolumePrune = false;
    await dockerStore.pruneTarget('container.docker.unused_volumes');
  }

  let isRefreshing = $state(false);

  async function handleRefresh() {
    if (isRefreshing) return;
    isRefreshing = true;
    const start = Date.now();
    await dockerStore.refresh();
    const elapsed = Date.now() - start;
    if (elapsed < 600) {
      await new Promise((r) => setTimeout(r, 600 - elapsed));
    }
    isRefreshing = false;
  }
</script>

<div class="space-y-6">
  <!-- Header Card -->
  <div class="flex items-center justify-between pb-3 border-b border-border/60">
    <div class="flex items-center gap-3">
      <div class="h-9 w-9 rounded-lg bg-cyan-500/10 text-cyan-400 flex items-center justify-center">
        <Container size={20} />
      </div>
      <div>
        <div class="flex items-center gap-2">
          <h2 class="text-base font-semibold text-foreground tracking-tight">Docker & Containers</h2>
          {#if status?.is_running}
            <Badge variant="success">Daemon Running</Badge>
          {:else if status?.is_available}
            <Badge variant="warning">Daemon Stopped</Badge>
          {:else}
            <Badge variant="secondary">Not Installed</Badge>
          {/if}
        </div>
        <p class="text-xs text-muted-foreground mt-0.5">
          {status?.version || 'Inspect and safely prune Docker containers, build cache, and dangling images.'}
        </p>
      </div>
    </div>

    <Button
      variant="outline"
      size="sm"
      disabled={isRefreshing || dockerStore.isLoading || dockerStore.isPruning}
      onclick={handleRefresh}
      class="gap-1.5 text-xs"
    >
      <RotateCw size={13} class={isRefreshing || dockerStore.isLoading ? 'animate-spin' : ''} />
      <span>Refresh</span>
    </Button>
  </div>

  {#if !status?.is_running}
    <Card class="p-8 text-center space-y-3 bg-secondary/30">
      <div class="h-12 w-12 rounded-full bg-muted flex items-center justify-center mx-auto text-muted-foreground">
        <Container size={24} />
      </div>
      <div class="space-y-1">
        <h3 class="text-sm font-semibold text-foreground">Docker Daemon is Inactive</h3>
        <p class="text-xs text-muted-foreground max-w-sm mx-auto">
          Start Docker Desktop or Colima to inspect images, containers, and build cache storage.
        </p>
      </div>
    </Card>
  {:else if overview}
    <!-- Storage Breakdown Grid -->
    <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
      <!-- Build Cache (Safe) -->
      <Card class="p-4 space-y-3 bg-card/60">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2 text-xs font-medium text-muted-foreground">
            <Layers size={15} class="text-purple-400" />
            <span>Build Cache</span>
          </div>
          <Badge variant="success">Safe</Badge>
        </div>
        <div>
          <div class="text-xl font-bold font-mono text-foreground">
            {formatBytes(overview.build_cache.reclaimable_bytes)}
          </div>
          <p class="text-[11px] text-muted-foreground mt-0.5">Unused BuildKit layers</p>
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={dockerStore.isPruning || overview.build_cache.reclaimable_bytes === 0}
          onclick={() => dockerStore.pruneTarget('container.docker.builder')}
          class="w-full text-xs gap-1.5 min-h-[30px]"
        >
          {#if dockerStore.isPruning}
            <DeletingDots size="xs" />
            <span>Pruning…</span>
          {:else}
            <Trash2 size={12} />
            <span>Prune Cache</span>
          {/if}
        </Button>
      </Card>

      <!-- Dangling Images (Safe) -->
      <Card class="p-4 space-y-3 bg-card/60">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2 text-xs font-medium text-muted-foreground">
            <Container size={15} class="text-emerald-400" />
            <span>Dangling Images</span>
          </div>
          <Badge variant="success">Safe</Badge>
        </div>
        <div>
          <div class="text-xl font-bold font-mono text-foreground">
            {formatBytes(overview.images.reclaimable_bytes)}
          </div>
          <p class="text-[11px] text-muted-foreground mt-0.5">Untagged layers</p>
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={dockerStore.isPruning || overview.images.reclaimable_bytes === 0}
          onclick={() => dockerStore.pruneTarget('container.docker.dangling_images')}
          class="w-full text-xs gap-1.5 min-h-[30px]"
        >
          {#if dockerStore.isPruning}
            <DeletingDots size="xs" />
            <span>Pruning…</span>
          {:else}
            <Trash2 size={12} />
            <span>Prune Images</span>
          {/if}
        </Button>
      </Card>

      <!-- Stopped Containers (Rebuild) -->
      <Card class="p-4 space-y-3 bg-card/60">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2 text-xs font-medium text-muted-foreground">
            <Server size={15} class="text-amber-400" />
            <span>Stopped Containers</span>
          </div>
          <Badge variant="warning">Rebuild</Badge>
        </div>
        <div>
          <div class="text-xl font-bold font-mono text-foreground">
            {formatBytes(overview.containers.reclaimable_bytes)}
          </div>
          <p class="text-[11px] text-muted-foreground mt-0.5">Exited container data</p>
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={dockerStore.isPruning || overview.containers.reclaimable_bytes === 0}
          onclick={() => dockerStore.pruneTarget('container.docker.stopped_containers')}
          class="w-full text-xs gap-1.5 min-h-[30px]"
        >
          {#if dockerStore.isPruning}
            <DeletingDots size="xs" />
            <span>Pruning…</span>
          {:else}
            <Trash2 size={12} />
            <span>Prune Containers</span>
          {/if}
        </Button>
      </Card>

      <!-- Unused Volumes (Manual) -->
      <Card class="p-4 space-y-3 bg-card/60">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2 text-xs font-medium text-muted-foreground">
            <HardDrive size={15} class="text-rose-400" />
            <span>Local Volumes</span>
          </div>
          <Badge variant="danger">Manual</Badge>
        </div>
        <div>
          <div class="text-xl font-bold font-mono text-foreground">
            {formatBytes(overview.volumes.reclaimable_bytes)}
          </div>
          <p class="text-[11px] text-muted-foreground mt-0.5">of {formatBytes(overview.volumes.total_bytes)} total</p>
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={dockerStore.isPruning || overview.volumes.reclaimable_bytes === 0}
          onclick={() => (confirmVolumePrune = true)}
          class="w-full text-xs gap-1.5 text-rose-400 hover:text-rose-400 min-h-[30px]"
        >
          {#if dockerStore.isPruning}
            <DeletingDots size="xs" />
            <span>Pruning…</span>
          {:else}
            <Trash2 size={12} />
            <span>Prune Volumes</span>
          {/if}
        </Button>
      </Card>
    </div>

    <!-- Active Containers & Images Table -->
    {#if status?.containers && status.containers.length > 0}
      <div class="space-y-3 pt-2">
        <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
          Detected Containers ({status.containers.length})
        </h3>
        <div class="space-y-2 max-h-60 overflow-y-auto pr-1">
          {#each status.containers as container}
            <div class="flex items-center justify-between p-3 rounded-lg bg-card/70 border border-border/60 text-xs">
              <div class="space-y-0.5">
                <div class="flex items-center gap-2 font-medium text-foreground">
                  <span>{container.name}</span>
                  <span class="text-muted-foreground font-mono text-[10px]">({container.image})</span>
                </div>
                <div class="flex items-center gap-2 text-[11px] text-muted-foreground">
                  <span class={container.is_running ? 'text-emerald-500 font-medium' : 'text-muted-foreground'}>
                    ● {container.state}
                  </span>
                </div>
              </div>
              <span class="font-mono text-muted-foreground">
                {formatBytes(container.size_bytes)}
              </span>
            </div>
          {/each}
        </div>
      </div>
    {/if}
  {/if}

  {#if confirmVolumePrune}
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-4 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="volume-prune-title">
      <Card class="w-full max-w-sm space-y-4 border-border bg-card p-5 shadow-2xl">
        <div class="flex items-start gap-3">
          <div class="mt-0.5 text-rose-500"><AlertCircle size={20} /></div>
          <div>
            <h3 id="volume-prune-title" class="text-sm font-semibold">Prune unused Docker volumes?</h3>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
              Docker reports {formatBytes(overview?.volumes.reclaimable_bytes ?? 0)} as reclaimable. Volumes may contain persistent application data and cannot be restored by Zenith.
            </p>
          </div>
        </div>
        <div class="flex justify-end gap-2">
          <Button variant="ghost" size="sm" onclick={() => (confirmVolumePrune = false)}>Cancel</Button>
          <Button variant="destructive" size="sm" onclick={pruneVolumes}>Prune Volumes</Button>
        </div>
      </Card>
    </div>
  {/if}
</div>
