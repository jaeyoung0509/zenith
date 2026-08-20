<script lang="ts">
  import { onMount } from 'svelte';
  import type { LocalModelItem } from '../../lib/models/types';
  import { localModelsStore } from '../../lib/stores/models.svelte';
  import { formatBytes, formatTimeAgo } from '../../lib/utils/format';
  import { tauriRevealInFinder } from '../../lib/utils/tauri';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import DeletingDots from '../../lib/components/DeletingDots.svelte';
  import {
    Boxes,
    RotateCw,
    Trash2,
    FolderOpen,
    AlertTriangle,
    Search,
  } from 'lucide-svelte';

  let models = $derived(localModelsStore.models);
  let totalBytes = $derived(localModelsStore.totalBytes);

  let searchQuery = $state('');
  let modelToDelete = $state<LocalModelItem | null>(null);

  onMount(() => {
    void localModelsStore.refresh();
  });

  let filteredModels = $derived.by(() => {
    if (!searchQuery.trim()) return models;
    const q = searchQuery.toLowerCase();
    return models.filter(
      (m) =>
        m.name.toLowerCase().includes(q) ||
        m.source.toLowerCase().includes(q) ||
        m.path.toLowerCase().includes(q)
    );
  });

  const sourceBadges = {
    ollama: { label: 'Ollama', variant: 'secondary' as const },
    huggingface: { label: 'HuggingFace', variant: 'warning' as const },
    lmstudio: { label: 'LM Studio', variant: 'default' as const },
    mlx: { label: 'Apple MLX', variant: 'success' as const },
  };

  function confirmDelete(model: LocalModelItem) {
    modelToDelete = model;
  }

  async function executeDeleteModel() {
    if (!modelToDelete) return;
    await localModelsStore.deleteModel(modelToDelete);
    modelToDelete = null;
  }

  let isRefreshing = $state(false);

  async function handleRefresh() {
    if (isRefreshing) return;
    isRefreshing = true;
    const start = Date.now();
    await localModelsStore.refresh();
    const elapsed = Date.now() - start;
    if (elapsed < 600) {
      await new Promise((r) => setTimeout(r, 600 - elapsed));
    }
    isRefreshing = false;
  }
</script>

<div class="space-y-6">
  <!-- Header Card -->
  <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4 pb-3 border-b border-border/60">
    <div class="flex items-center gap-3">
      <div class="h-9 w-9 rounded-lg bg-amber-500/10 text-amber-400 flex items-center justify-center">
        <Boxes size={20} />
      </div>
      <div>
        <div class="flex items-center gap-2">
          <h2 class="text-base font-semibold text-foreground tracking-tight">Local LLM Models</h2>
          <Badge variant="outline" class="font-mono">{formatBytes(totalBytes)}</Badge>
        </div>
        <p class="text-xs text-muted-foreground mt-0.5">
          Ollama, HuggingFace Hub, LM Studio, and Apple MLX downloaded model weights.
        </p>
      </div>
    </div>

    <div class="flex items-center gap-2">
      <Button
        variant="outline"
        size="sm"
        disabled={isRefreshing || localModelsStore.isLoading}
        onclick={handleRefresh}
        class="gap-1.5 text-xs"
      >
        <RotateCw size={13} class={isRefreshing || localModelsStore.isLoading ? 'animate-gentle-spin' : ''} />
        <span>Rescan Models</span>
      </Button>
    </div>
  </div>

  {#if localModelsStore.error}
    <div class="rounded-xl border border-rose-500/20 bg-rose-500/5 px-4 py-3 text-xs text-rose-500 flex items-center justify-between">
      <span>{localModelsStore.error}</span>
      <Button variant="ghost" size="sm" onclick={() => (localModelsStore.error = null)} class="text-xs h-6 px-2 text-rose-400">Dismiss</Button>
    </div>
  {/if}

  <!-- Search Toolbar -->
  <div class="relative w-full sm:w-80">
    <Search
      size={14}
      class="absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground pointer-events-none"
    />
    <input
      type="text"
      bind:value={searchQuery}
      placeholder="Search local models..."
      class="w-full h-8 pl-8 pr-3 text-xs rounded-lg border border-border bg-card text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
    />
  </div>

  <!-- Models List -->
  {#if localModelsStore.isLoading}
    <div class="py-16 text-center text-xs text-muted-foreground space-y-2">
      <RotateCw size={20} class="animate-spin mx-auto opacity-50" />
      <p>Discovering installed local models...</p>
    </div>
  {:else if filteredModels.length > 0}
    <div class="space-y-2.5">
      {#each filteredModels as model (model.id)}
        <div
          class="flex items-center justify-between p-3.5 rounded-xl border border-border/70 bg-card/70 hover:border-border transition-colors group"
        >
          <div class="space-y-1 flex-1 min-w-0 pr-3">
            <div class="flex items-center gap-2">
              <span class="text-xs font-semibold text-foreground truncate">{model.name}</span>
              <Badge variant={sourceBadges[model.source]?.variant || 'secondary'}>
                {sourceBadges[model.source]?.label || model.source}
              </Badge>
              {#if model.format}
                <span class="text-[10px] text-muted-foreground font-mono bg-secondary/80 px-1.5 py-0.5 rounded">
                  {model.format}
                </span>
              {/if}
            </div>

            <div class="flex items-center gap-2 text-[11px] text-muted-foreground font-mono">
              <span class="truncate max-w-[320px]">{model.path}</span>
              {#if model.last_modified}
                <span>• modified {formatTimeAgo(model.last_modified)}</span>
              {/if}
            </div>
          </div>

          <div class="flex items-center gap-3 shrink-0">
            <span class="text-xs font-mono font-bold text-foreground">
              {formatBytes(model.size_bytes)}
            </span>

            <Button
              variant="ghost"
              size="icon"
              class="h-7 w-7 text-muted-foreground"
              onclick={() => tauriRevealInFinder(model.path)}
            >
              <FolderOpen size={13} />
            </Button>

            <Button
              variant="outline"
              size="sm"
              class="h-7 px-2 text-xs text-rose-500 hover:text-rose-500 hover:border-rose-500/30 gap-1"
              onclick={() => confirmDelete(model)}
            >
              <Trash2 size={12} />
              <span>Delete</span>
            </Button>
          </div>
        </div>
      {/each}
    </div>
  {:else}
    <Card class="p-12 text-center text-xs text-muted-foreground space-y-2 bg-secondary/20">
      <Boxes size={24} class="mx-auto opacity-40" />
      <p class="font-medium text-foreground">No local models detected</p>
      <p>Ollama, HuggingFace Hub, or LM Studio model weights will appear here once downloaded.</p>
    </Card>
  {/if}

  <!-- Delete Confirmation Modal -->
  {#if modelToDelete}
    <div class="fixed inset-0 z-50 bg-background/80 backdrop-blur-sm flex items-center justify-center p-4">
      <Card class="w-full max-w-sm bg-card border-border shadow-2xl p-5 space-y-4">
        <div class="flex items-center gap-2.5 text-amber-500">
          <AlertTriangle size={20} />
          <h3 class="text-sm font-semibold text-foreground">Delete Model Weights?</h3>
        </div>

        <p class="text-xs text-muted-foreground leading-relaxed">
          Are you sure you want to delete <span class="font-semibold text-foreground">{modelToDelete.name}</span> ({formatBytes(modelToDelete.size_bytes)})? You will need to re-download this model if you want to use it again.
        </p>

        <div class="flex items-center justify-end gap-2 pt-2">
          <Button variant="ghost" size="sm" onclick={() => (modelToDelete = null)}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            size="sm"
            disabled={localModelsStore.isDeleting}
            onclick={executeDeleteModel}
            class="min-w-[95px] gap-1.5"
          >
            {#if localModelsStore.isDeleting}
              <DeletingDots size="xs" />
              <span>Deleting…</span>
            {:else}
              <Trash2 size={12} />
              <span>Delete Model</span>
            {/if}
          </Button>
        </div>
      </Card>
    </div>
  {/if}
</div>
