<script lang="ts">
  import { onMount } from 'svelte';
  import { usageStore } from '../../lib/stores/usage.svelte';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import ProgressBar from '../../lib/components/ProgressBar.svelte';
  import { Activity, Bot, RefreshCw, ShieldCheck, Terminal, Zap } from 'lucide-svelte';

  onMount(() => usageStore.refresh());

  const compactNumber = new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 });

  function formatTokens(value?: number) {
    return value == null ? '—' : compactNumber.format(value);
  }

  function formatReset(timestamp?: number) {
    if (!timestamp) return '';
    return new Intl.DateTimeFormat('en', {
      month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit',
    }).format(new Date(timestamp * 1000));
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
    <Button variant="outline" size="sm" class="gap-1.5" disabled={usageStore.isLoading} onclick={() => usageStore.refresh()}>
      <RefreshCw size={13} class={usageStore.isLoading ? 'animate-spin' : ''} />
      Refresh
    </Button>
  </div>

  <div class="flex items-start gap-2.5 rounded-xl border border-emerald-500/20 bg-emerald-500/5 px-4 py-3">
    <ShieldCheck size={16} class="text-emerald-400 mt-0.5 shrink-0" />
    <p class="text-xs text-muted-foreground leading-relaxed">
      Zenith asks official local clients for usage metadata. OAuth token files are never read or sent to the UI.
    </p>
  </div>

  {#if usageStore.error}
    <div class="rounded-xl border border-red-500/20 bg-red-500/5 p-4 text-xs text-red-400">{usageStore.error}</div>
  {:else if usageStore.isLoading && !usageStore.snapshot}
    <div class="py-20 text-center text-muted-foreground">
      <RefreshCw size={22} class="animate-spin mx-auto mb-3" />
      <p class="text-xs">Reading connected AI accounts…</p>
    </div>
  {:else if usageStore.snapshot}
    <div class="grid grid-cols-2 gap-3">
      {#each usageStore.snapshot.providers as provider}
        <Card class="p-4 min-h-[190px] flex flex-col">
          <div class="flex items-start justify-between gap-3">
            <div class="flex items-center gap-2.5">
              <div class="h-8 w-8 rounded-lg bg-secondary flex items-center justify-center text-foreground">
                {#if provider.id === 'codex'}<Zap size={16} />
                {:else if provider.id === 'opencode'}<Terminal size={16} />
                {:else}<Bot size={16} />{/if}
              </div>
              <div>
                <h3 class="text-sm font-semibold">{provider.name}</h3>
                <p class="text-[10px] text-muted-foreground">{provider.auth_label}</p>
              </div>
            </div>
            <span class="text-[10px] px-2 py-0.5 rounded-full border {provider.connected ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-400' : provider.support === 'local' ? 'border-blue-500/30 bg-blue-500/10 text-blue-400' : 'border-border text-muted-foreground'}">
              {provider.connected ? 'Connected' : provider.support === 'manual' ? 'Manual' : 'Available'}
            </span>
          </div>

          {#if provider.windows.length}
            <div class="mt-4 space-y-3">
              {#each provider.windows as window}
                <div class="space-y-1.5">
                  <div class="flex justify-between text-[11px]">
                    <span class="text-muted-foreground">{window.label}</span>
                    <span class="font-mono">{Math.round(window.used_percent)}% used</span>
                  </div>
                  <ProgressBar value={window.used_percent} height="h-1.5" />
                  <p class="text-[9px] text-muted-foreground text-right">Resets {formatReset(window.resets_at)}</p>
                </div>
              {/each}
            </div>
          {:else if provider.summary.local_sessions != null}
            <div class="mt-4 grid grid-cols-2 gap-2">
              <div class="rounded-lg bg-secondary/50 p-2.5"><p class="text-[9px] text-muted-foreground uppercase">7d sessions</p><p class="text-lg font-mono font-semibold">{provider.summary.local_sessions}</p></div>
              <div class="rounded-lg bg-secondary/50 p-2.5"><p class="text-[9px] text-muted-foreground uppercase">Local cost</p><p class="text-lg font-mono font-semibold">${(provider.summary.local_cost_usd ?? 0).toFixed(2)}</p></div>
            </div>
          {:else if provider.id === 'openrouter' && provider.connected}
            <div class="mt-4 grid grid-cols-2 gap-2">
              <div class="rounded-lg bg-secondary/50 p-2.5"><p class="text-[9px] text-muted-foreground uppercase">Total usage</p><p class="text-lg font-mono font-semibold">${(provider.summary.usage_usd ?? 0).toFixed(2)}</p></div>
              <div class="rounded-lg bg-secondary/50 p-2.5"><p class="text-[9px] text-muted-foreground uppercase">Limit remaining</p><p class="text-lg font-mono font-semibold">{provider.summary.limit_remaining_usd == null ? '—' : `$${provider.summary.limit_remaining_usd.toFixed(2)}`}</p></div>
            </div>
          {:else}
            <div class="flex-1 flex items-center py-4">
              <p class="text-[11px] leading-relaxed text-muted-foreground">{provider.status_message}</p>
            </div>
          {/if}

          {#if provider.id === 'openrouter' && !provider.connected}
            <Button
              variant="outline"
              size="sm"
              class="mt-auto w-full gap-1.5"
              disabled={usageStore.connectingProvider === 'openrouter'}
              onclick={() => usageStore.connectOpenRouter()}
            >
              {usageStore.connectingProvider === 'openrouter' ? 'Waiting for browser…' : 'Connect with OpenRouter'}
            </Button>
          {/if}

          {#if provider.id === 'codex' && provider.summary.lifetime_tokens != null}
            <div class="mt-auto pt-3 border-t border-border/50 grid grid-cols-3 gap-2 text-center">
              <div><p class="text-[9px] text-muted-foreground">Lifetime</p><p class="text-xs font-mono font-medium">{formatTokens(provider.summary.lifetime_tokens)}</p></div>
              <div><p class="text-[9px] text-muted-foreground">Recent 7 days</p><p class="text-xs font-mono font-medium">{formatTokens(provider.summary.last_7d_tokens)}</p></div>
              <div><p class="text-[9px] text-muted-foreground">Streak</p><p class="text-xs font-mono font-medium">{provider.summary.current_streak_days ?? '—'}d</p></div>
            </div>
          {/if}
        </Card>
      {/each}
    </div>
  {/if}
</div>
