<script lang="ts">
  import { onMount } from 'svelte';
  import { usageStore } from '../../lib/stores/usage.svelte';
  import { withMinimumDuration } from '../../lib/utils/async';
  import Button from '../../lib/components/Button.svelte';
  import AiUsageCards from '../../lib/components/AiUsageCards.svelte';
  import { Activity, RefreshCw, ShieldCheck } from 'lucide-svelte';

  onMount(() => usageStore.refreshIfStale());

  let isRefreshing = $state(false);

  async function handleRefresh() {
    if (isRefreshing) return;
    isRefreshing = true;
    try {
      await withMinimumDuration(usageStore.refresh(true), 600);
    } finally {
      isRefreshing = false;
    }
  }
</script>

<div class="space-y-6">
  <div class="flex items-center justify-between pb-3 border-b border-border/60">
    <div class="flex items-center gap-3">
      <div class="h-9 w-9 rounded-lg bg-violet-500/10 text-violet-400 flex items-center justify-center">
        <Activity size={20} />
      </div>
      <div>
        <h2 class="text-base font-semibold text-foreground tracking-tight">AI Usage</h2>
        <p class="text-xs text-muted-foreground mt-0.5">OAuth account limits and local coding-agent activity</p>
      </div>
    </div>
    <Button variant="outline" size="sm" class="gap-1.5" disabled={isRefreshing || usageStore.isLoading} onclick={handleRefresh}>
      <RefreshCw size={13} class={isRefreshing || usageStore.isLoading ? 'animate-gentle-spin' : ''} />
      Refresh
    </Button>
  </div>

  <div class="flex items-start gap-2.5 rounded-xl border border-success/20 bg-success/5 px-4 py-3">
    <ShieldCheck size={16} class="text-success mt-0.5 shrink-0" />
    <p class="text-xs text-muted-foreground leading-relaxed">
      Zenith asks official local clients for usage metadata. OAuth token files are never read or sent to the UI.
    </p>
  </div>

  {#if usageStore.error}
    <div class="rounded-xl border border-destructive/20 bg-destructive/5 p-4 text-xs text-destructive">{usageStore.error}</div>
  {:else if usageStore.isLoading && !usageStore.snapshot}
    <div class="py-20 text-center text-muted-foreground">
      <RefreshCw size={22} class="animate-gentle-spin mx-auto mb-3" />
      <p class="text-xs">Reading connected AI accounts…</p>
    </div>
  {:else if usageStore.snapshot}
    <AiUsageCards
      providers={usageStore.snapshot.providers}
      connectingProvider={usageStore.connectingProvider}
      onConnectOpenRouter={() => usageStore.connectOpenRouter()}
    />
  {/if}
</div>
