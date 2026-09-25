<script lang="ts">
  import type { AiProviderUsage } from '../models/types';
  import { formatResetDate, formatTimeUntil } from '../utils/format';
  import Button from './Button.svelte';
  import Card from './Card.svelte';
  import ProgressBar from './ProgressBar.svelte';

  interface Props {
    providers: readonly AiProviderUsage[];
    isProviderLoading?: (id: string) => boolean;
    connectingProvider?: string | null;
    onConnectOpenRouter?: () => void | Promise<void>;
    onDisconnectOpenRouter?: () => void | Promise<void>;
  }

  let {
    providers,
    isProviderLoading = () => false,
    connectingProvider = null,
    onConnectOpenRouter,
    onDisconnectOpenRouter,
  }: Props = $props();

  const compactNumber = new Intl.NumberFormat('en', {
    notation: 'compact',
    maximumFractionDigits: 1,
  });

  function formatTokens(value?: number | null) {
    return value == null ? '—' : compactNumber.format(value);
  }
</script>

<div class="grid grid-cols-1 gap-3 md:grid-cols-2">
  {#each providers as provider (provider.id)}
    {@const loading = isProviderLoading(provider.id)}
    <Card class="flex min-h-[190px] flex-col p-4">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <div class="min-w-0">
            <h3 class="truncate text-body font-semibold">
              {provider.name}{provider.model_identity ? ` · ${provider.model_identity}` : ''}
            </h3>
            <p class="truncate text-caption text-muted-foreground">{provider.auth_label}</p>
          </div>
        </div>
        {#if loading}
          <span class="h-5 w-16 shrink-0 animate-pulse rounded-full bg-secondary" aria-hidden="true"></span>
        {:else}
          <span class="shrink-0 text-caption px-2 py-0.5 rounded-md border {provider.connected ? 'border-success/25 bg-success/10 text-success' : provider.support === 'local' ? 'border-ai/25 bg-ai/10 text-ai' : 'border-border text-muted-foreground'}">
            {provider.connected ? 'Connected' : provider.support === 'manual' ? 'Manual' : 'Available'}
          </span>
        {/if}
      </div>

      {#if loading}
        <div
          class="mt-4 flex-1 space-y-4 animate-pulse"
          role="status"
          aria-label={`Loading ${provider.name} usage`}
        >
          <div class="space-y-2">
            <div class="flex items-center justify-between gap-4">
              <span class="h-2.5 w-16 rounded bg-secondary"></span>
              <span class="h-2.5 w-14 rounded bg-secondary"></span>
            </div>
            <div class="h-1.5 w-full rounded-full bg-secondary"></div>
            <div class="ml-auto h-2 w-28 rounded bg-secondary"></div>
          </div>
          <div class="grid grid-cols-3 gap-2 border-t border-border pt-3">
            {#each Array(3) as _}
              <div class="space-y-1.5">
                <div class="mx-auto h-2 w-10 rounded bg-secondary"></div>
                <div class="mx-auto h-3 w-8 rounded bg-secondary"></div>
              </div>
            {/each}
          </div>
          <span class="sr-only">Loading usage metadata…</span>
        </div>
      {:else if provider.windows.length}
        <div class="mt-4 space-y-3">
          {#each provider.windows as usageWindow}
            <div class="space-y-1.5">
              <div class="flex justify-between gap-3 text-meta">
                <span class="truncate text-muted-foreground">{usageWindow.label}</span>
                <span class="shrink-0 font-mono">
                  {usageWindow.used_percent != null ? `${Math.round(usageWindow.used_percent)}% used` : '—'}
                </span>
              </div>
              <ProgressBar value={usageWindow.used_percent ?? 0} height="h-1.5" />
              {#if usageWindow.resets_at}
                <p class="text-right text-caption text-muted-foreground" title={formatResetDate(usageWindow.resets_at)}>
                  Resets {formatResetDate(usageWindow.resets_at)}
                  {#if formatTimeUntil(usageWindow.resets_at)}
                    <span> · in {formatTimeUntil(usageWindow.resets_at)}</span>
                  {/if}
                </p>
              {/if}
            </div>
          {/each}
        </div>
      {:else if provider.summary.local_sessions != null}
        <div class="mt-4 grid grid-cols-2 gap-2">
          <div class="rounded-lg bg-secondary p-2.5">
            <p class="text-caption text-muted-foreground uppercase">7d sessions</p>
            <p class="text-title font-mono font-semibold tabular-nums">{provider.summary.local_sessions}</p>
          </div>
          <div class="rounded-lg bg-secondary p-2.5">
            <p class="text-caption text-muted-foreground uppercase">Local cost</p>
            <p class="text-title font-mono font-semibold tabular-nums">${(provider.summary.local_cost_usd ?? 0).toFixed(2)}</p>
          </div>
        </div>
      {:else if provider.id === 'openrouter' && provider.connected}
        <div class="mt-4 grid grid-cols-2 gap-2">
          <div class="rounded-lg bg-secondary p-2.5">
            <p class="text-caption text-muted-foreground uppercase">Total usage</p>
            <p class="text-title font-mono font-semibold tabular-nums">${(provider.summary.usage_usd ?? 0).toFixed(2)}</p>
          </div>
          <div class="rounded-lg bg-secondary p-2.5">
            <p class="text-caption text-muted-foreground uppercase">Limit remaining</p>
            <p class="text-title font-mono font-semibold tabular-nums">
              {provider.summary.limit_remaining_usd == null ? '—' : `$${provider.summary.limit_remaining_usd.toFixed(2)}`}
            </p>
          </div>
        </div>
      {:else}
        <div class="flex-1 flex items-center py-4">
          <p class="text-meta leading-relaxed text-muted-foreground">{provider.status_message}</p>
        </div>
      {/if}

      {#if !loading && provider.id === 'openrouter' && !provider.connected && onConnectOpenRouter}
        <Button
          variant="outline"
          size="sm"
          class="mt-auto w-full gap-1.5"
          disabled={connectingProvider === 'openrouter'}
          onclick={onConnectOpenRouter}
        >
          {connectingProvider === 'openrouter' ? 'Waiting for browser…' : 'Connect with OpenRouter'}
        </Button>
      {:else if !loading && provider.id === 'openrouter' && provider.connected && onDisconnectOpenRouter}
        <div class="mt-auto space-y-1.5">
          <p class="text-caption text-muted-foreground">
            Disconnecting removes the key from Zenith. Revoke it in the OpenRouter dashboard to invalidate it everywhere.
          </p>
          <Button
            variant="outline"
            size="sm"
            class="w-full gap-1.5"
            disabled={connectingProvider === 'openrouter'}
            onclick={onDisconnectOpenRouter}
          >
            {connectingProvider === 'openrouter' ? 'Disconnecting…' : 'Disconnect OpenRouter'}
          </Button>
        </div>
      {/if}

      {#if !loading && provider.id === 'codex' && provider.summary.lifetime_tokens != null}
        <div class="mt-auto grid grid-cols-3 gap-2 border-t border-border pt-3 text-center">
          <div><p class="text-caption text-muted-foreground">Lifetime</p><p class="text-meta font-mono font-medium tabular-nums">{formatTokens(provider.summary.lifetime_tokens)}</p></div>
          <div><p class="text-caption text-muted-foreground">Recent 7 days</p><p class="text-meta font-mono font-medium tabular-nums">{formatTokens(provider.summary.last_7d_tokens)}</p></div>
          <div><p class="text-caption text-muted-foreground">Streak</p><p class="text-meta font-mono font-medium tabular-nums">{provider.summary.current_streak_days ?? '—'}d</p></div>
        </div>
      {/if}
    </Card>
  {/each}
</div>
