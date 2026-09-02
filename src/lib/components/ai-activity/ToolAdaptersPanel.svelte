<script lang="ts">
  import type { AgentAdapterState, AgentEvidence } from '../../models/types';
  import { agentActivityStore } from '../../stores/agentActivity.svelte';
  import Badge from '../Badge.svelte';
  import Button from '../Button.svelte';
  import Card from '../Card.svelte';

  let snapshot = $derived(agentActivityStore.snapshot);
  let integrations = $derived(agentActivityStore.integrations);
  let actionFeedback = $state<string | null>(null);

  function adapterStatusBadge(stateValue: AgentAdapterState) {
    switch (stateValue) {
      case 'connected':
        return { label: 'Connected', variant: 'success' as const };
      case 'integration_available':
        return { label: 'Integration Available', variant: 'warning' as const };
      case 'process_only':
        return { label: 'Process Only', variant: 'secondary' as const };
      case 'not_installed':
        return { label: 'Not Installed', variant: 'outline' as const };
      case 'version_unsupported':
        return { label: 'Version Unsupported', variant: 'danger' as const };
      case 'partial':
        return { label: 'Partial', variant: 'warning' as const };
    }
  }

  function evidenceBadge(evidence: AgentEvidence | null) {
    if (evidence === 'vendor_confirmed' || evidence === 'vendor_event' || evidence === 'vendor_protocol') {
      return { label: 'Vendor confirmed', variant: 'success' as const };
    }
    if (evidence === 'heuristic') {
      return { label: 'Heuristic', variant: 'warning' as const };
    }
    if (evidence === 'process_observed') {
      return { label: 'Process observed', variant: 'secondary' as const };
    }
    return null;
  }

  async function handleUninstallIntegration(toolId: string) {
    try {
      const result = await agentActivityStore.uninstallIntegration(toolId);
      actionFeedback = result.message;
    } catch (err) {
      actionFeedback = `Uninstall failed: ${err instanceof Error ? err.message : String(err)}`;
    }
  }
</script>

<div
  id="ai-activity-panel-adapters"
  role="tabpanel"
  aria-labelledby="ai-activity-tab-adapters"
  tabindex="0"
  class="space-y-4 outline-none focus-visible:ring-1 focus-visible:ring-ring"
>
  <div>
    <h3 class="text-sm font-semibold">Tool Adapters</h3>
    <p class="text-caption text-muted-foreground">
      Exact local process observation across supported AI developer tools.
    </p>
  </div>

  {#if agentActivityStore.error}
    <div role="alert" class="rounded-xl border border-destructive/20 bg-destructive/5 px-4 py-3 text-xs text-destructive">
      Tool adapter refresh failed. The last successful local snapshot remains visible. {agentActivityStore.error}
    </div>
  {/if}

  {#if agentActivityStore.integrationsError}
    <div role="alert" class="rounded-xl border border-warning/20 bg-warning/5 px-4 py-3 text-xs text-warning">
      Integration status unavailable. Adapter process evidence remains visible. {agentActivityStore.integrationsError}
    </div>
  {/if}

  {#if agentActivityStore.isIntegrationsLoading}
    <div role="status" aria-label="Loading tool adapter integrations" class="text-caption text-muted-foreground">
      Loading integration status…
    </div>
  {/if}

  {#if !snapshot && agentActivityStore.isLoading}
    <div aria-label="Loading tool adapters" class="grid grid-cols-1 gap-3 md:grid-cols-2">
      {#each Array(4) as _}
        <div class="h-28 animate-pulse rounded-xl border border-border/60 bg-secondary/30"></div>
      {/each}
    </div>
  {:else if snapshot && snapshot.adapters.length > 0}
    {#if snapshot.partial_errors.length > 0}
      <div role="status" class="rounded-xl border border-warning/20 bg-warning/5 px-4 py-3 text-xs text-warning">
        Partial adapter snapshot: {snapshot.partial_errors.join(' · ')}
      </div>
    {/if}

    <div class="grid grid-cols-1 gap-3 md:grid-cols-2">
      {#each snapshot.adapters as adapter (adapter.tool_id)}
        {@const status = adapterStatusBadge(adapter.state)}
        {@const evidence = evidenceBadge(adapter.evidence)}
        {@const integration = integrations.find((item) => item.tool_id === adapter.tool_id)}
        <Card class="p-4 bg-card/60 border-border/70 space-y-2.5">
          <div class="flex items-start justify-between gap-2">
            <div class="min-w-0">
              <div class="flex items-center gap-2 flex-wrap">
                <span class="text-xs font-bold">{adapter.display_name}</span>
                <Badge variant={status.variant}>{status.label}</Badge>
                {#if evidence}
                  <Badge variant={evidence.variant}>{evidence.label}</Badge>
                {/if}
              </div>
              <p class="text-caption text-muted-foreground mt-1">{adapter.message}</p>
            </div>

            {#if integration?.integration_active}
              <Button
                variant="outline"
                size="sm"
                class="text-caption text-destructive hover:bg-destructive/10"
                onclick={() => handleUninstallIntegration(adapter.tool_id)}
                title={`Remove legacy marker for ${adapter.display_name}`}
              >
                Remove legacy marker
              </Button>
            {/if}
          </div>

          {#if integration?.integration_active && integration.config_path}
            <div class="font-mono text-caption text-muted-foreground truncate pt-1 border-t border-border/40">
              Config: {integration.config_path}
            </div>
          {/if}
        </Card>
      {/each}
    </div>
  {:else if snapshot}
    <Card class="p-8 text-center bg-card/60">
      <p class="text-xs text-muted-foreground">No supported tool adapters were detected.</p>
    </Card>
  {:else}
    <Card class="p-8 text-center bg-card/60">
      <p class="text-xs text-muted-foreground">No tool adapter snapshot is available yet.</p>
    </Card>
  {/if}

  {#if actionFeedback}
    <div role="status" class="rounded-xl border border-primary/20 bg-primary/5 px-4 py-3 text-xs text-primary">
      {actionFeedback}
    </div>
  {/if}
</div>
