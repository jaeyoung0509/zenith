<script lang="ts">
  import AiUsageCards from '../AiUsageCards.svelte';
  import Card from '../Card.svelte';
  import { usageStore } from '../../stores/usage.svelte';

  let usageSnapshot = $derived(usageStore.snapshot);
  let providers = $derived(usageStore.providers);
  let connectedCount = $derived(providers.filter((provider) => provider.connected).length);
</script>

<div
  id="ai-activity-panel-usage"
  role="tabpanel"
  aria-labelledby="ai-activity-tab-usage"
  tabindex="0"
  class="space-y-4 outline-none focus-visible:ring-1 focus-visible:ring-ring"
>
  <div class="flex items-center justify-between gap-3">
    <div>
      <h3 class="text-sm font-semibold">AI Accounts &amp; Quota</h3>
      <p class="text-caption text-muted-foreground">
        Official-client usage metadata and local coding-agent activity. OAuth token files never reach the UI.
      </p>
    </div>
    {#if usageSnapshot}
      <span class="shrink-0 text-caption text-muted-foreground">{connectedCount} connected</span>
    {/if}
  </div>

  {#if usageStore.error}
    <div role="alert" class="rounded-xl border border-destructive/20 bg-destructive/5 px-4 py-3 text-xs text-destructive">
      Usage refresh failed. The last successful usage snapshot remains visible. {usageStore.error}
    </div>
  {/if}

  {#if providers.length > 0}
    <AiUsageCards
      providers={providers}
      isProviderLoading={(id) => usageStore.isProviderLoading(id)}
      connectingProvider={usageStore.connectingProvider}
      onConnectOpenRouter={() => usageStore.connectOpenRouter()}
    />
  {:else if usageStore.isLoading}
    <div role="status" aria-label="Loading usage metadata">
      <Card class="p-8 text-center bg-card/60">
        <div class="mx-auto h-6 w-32 animate-pulse rounded bg-secondary" aria-hidden="true"></div>
        <p class="mt-3 text-xs text-muted-foreground">Loading usage metadata…</p>
      </Card>
    </div>
  {:else}
    <Card class="p-4 bg-card/60 text-xs text-muted-foreground">
      No AI account usage metadata is available yet.
    </Card>
  {/if}
</div>
