<script lang="ts">
  import { onMount, tick } from 'svelte';
  import type { DevelopmentListener } from '../../lib/models/types';
  import {
    developmentPortsStore,
    filterDevelopmentListeners,
  } from '../../lib/stores/developmentPorts.svelte';
  import { platformCapabilitiesStore } from '../../lib/stores/platformCapabilities.svelte';
  import { formatProcessAge } from '../../lib/utils/format';
  import { withMinimumDuration } from '../../lib/utils/async';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import InlineNotice from '../../lib/components/InlineNotice.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import {
    Globe,
    LogOut,
    Radio,
    RotateCw,
    Search,
    Server,
    ShieldCheck,
    TriangleAlert,
    X,
  } from 'lucide-svelte';

  let searchQuery = $state('');
  let isRefreshing = $state(false);
  let pendingReleaseListener = $state<DevelopmentListener | null>(null);
  let pendingForceListener = $state<DevelopmentListener | null>(null);
  let releaseReturnFocusId = $state<string | null>(null);
  let isWindows = $derived(platformCapabilitiesStore.capabilities?.platform === 'windows');

  let filteredListeners = $derived(
    filterDevelopmentListeners(developmentPortsStore.listeners, searchQuery)
  );

  onMount(() => {
    function updatePolling() {
      if (typeof document !== 'undefined' && document.visibilityState === 'visible') {
        developmentPortsStore.startPolling(15000);
      } else {
        developmentPortsStore.stopPolling();
      }
    }

    updatePolling();
    document.addEventListener('visibilitychange', updatePolling);

    return () => {
      document.removeEventListener('visibilitychange', updatePolling);
      developmentPortsStore.stopPolling();
    };
  });

  async function handleRefresh() {
    if (isRefreshing) return;
    isRefreshing = true;
    try {
      await withMinimumDuration(developmentPortsStore.refresh(), 500);
    } finally {
      isRefreshing = false;
    }
  }

  async function handleReleaseNormally() {
    if (!pendingReleaseListener) return;
    const listener = pendingReleaseListener;
    pendingReleaseListener = null;
    await restoreReleaseFocus();

    try {
      const result = await developmentPortsStore.release(listener, 'graceful');
      if (result.outcome === 'still_listening' && result.listener) {
        pendingForceListener = result.listener;
        await focusDialog('force-release-cancel');
      }
    } catch {
      // The store exposes the error in the page-level status message.
    }
  }

  async function handleForceRelease() {
    if (!pendingForceListener) return;
    const listener = pendingForceListener;
    pendingForceListener = null;
    await restoreReleaseFocus();

    try {
      await developmentPortsStore.release(listener, 'force');
    } catch {
      // The store exposes the error in the page-level status message.
    }
  }

  function releaseButtonId(listener: DevelopmentListener) {
    const address = listener.bind_address.replace(/[^a-zA-Z0-9]/g, '-');
    return `release-port-${listener.pid}-${listener.port}-${address}`;
  }

  function openReleaseDialog(listener: DevelopmentListener) {
    releaseReturnFocusId = releaseButtonId(listener);
    pendingReleaseListener = isWindows ? null : listener;
    pendingForceListener = isWindows ? listener : null;
    void focusDialog(isWindows ? 'force-release-cancel' : 'release-cancel');
  }

  async function focusDialog(id: string) {
    await tick();
    document.getElementById(id)?.focus();
  }

  async function restoreReleaseFocus() {
    await tick();
    if (releaseReturnFocusId) document.getElementById(releaseReturnFocusId)?.focus();
  }

  function closeDialogs() {
    pendingReleaseListener = null;
    pendingForceListener = null;
    void restoreReleaseFocus();
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && (pendingReleaseListener || pendingForceListener)) {
      closeDialogs();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="space-y-6">
  <!-- Page Header -->
  <PageHeader
    title="Development Servers"
    subtitle="Inspect local development and testing ports, then release one verified listener at a time."
    icon={Server}
    badge="TCP Listeners"
  >
    {#snippet actions()}
      <Button
        variant="outline"
        size="sm"
        disabled={isRefreshing || developmentPortsStore.isLoading}
        onclick={handleRefresh}
        class="gap-1.5 text-xs"
        title="Refresh development server listeners"
      >
        <RotateCw size={13} class={isRefreshing || developmentPortsStore.isLoading ? 'animate-gentle-spin' : ''} />
        <span>Refresh</span>
      </Button>
    {/snippet}
  </PageHeader>

  <div class="flex justify-end">
    <div class="relative w-full sm:w-72">
      <Search size={14} class="absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
      <input
        type="text"
        bind:value={searchQuery}
        placeholder="Search port, server, project…"
        aria-label="Search development server ports"
        class="h-8 w-full rounded-lg border border-border bg-card pl-8 pr-7 text-xs text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
      />
      {#if searchQuery}
        <button
          type="button"
          onclick={() => (searchQuery = '')}
          class="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
          aria-label="Clear port search"
        >
          <X size={13} />
        </button>
      {/if}
    </div>
  </div>

  {#if developmentPortsStore.error}
    <InlineNotice
      variant="destructive"
      title="Development Server Error"
      message={developmentPortsStore.error}
      onDismiss={() => developmentPortsStore.clearError()}
    />
  {:else if developmentPortsStore.lastAction}
    <InlineNotice
      variant="success"
      title="Action Complete"
      message={developmentPortsStore.lastAction}
      onDismiss={() => developmentPortsStore.clearLastAction()}
    />
  {/if}

  {#if filteredListeners.length > 0}
    <div class="divide-y divide-border/60 overflow-hidden rounded-xl border border-border/80 bg-card/70">
      {#each filteredListeners as listener (listener.id)}
        <div class="group flex flex-col justify-between gap-2.5 p-3 text-xs transition-colors hover:bg-secondary/30 sm:flex-row sm:items-center">
          <div class="flex min-w-0 items-center gap-3">
            <div class="w-20 shrink-0 font-mono text-sm font-bold text-foreground">
              {listener.port}<span class="ml-0.5 text-caption font-normal text-muted-foreground">/TCP</span>
            </div>
            <div class="min-w-0 space-y-0.5">
              <div class="flex flex-wrap items-center gap-2">
                <span class="font-semibold text-foreground">{listener.server_name}</span>
                {#if listener.project_name}
                  <span class="rounded bg-secondary px-1.5 py-0.5 text-caption font-medium text-muted-foreground">{listener.project_name}</span>
                {/if}
                <span class="font-mono text-meta text-muted-foreground">PID {listener.pid}</span>
              </div>
              {#if listener.working_directory}
                <p class="truncate font-mono text-meta text-muted-foreground" title={listener.working_directory}>{listener.working_directory}</p>
              {/if}
            </div>
          </div>

          <div class="flex shrink-0 items-center justify-between gap-3 pt-1 sm:justify-end sm:pt-0">
            <div class="flex items-center gap-1.5 font-mono text-meta">
              <span class="text-muted-foreground">{listener.bind_address}</span>
              {#if listener.exposure === 'all_interfaces'}
                <span class="inline-flex items-center gap-1 rounded border border-warning/20 bg-warning/10 px-1.5 py-0.5 text-meta font-medium text-warning" title="Exposed to all local and external network interfaces">
                  <TriangleAlert size={10} /> All interfaces
                </span>
              {:else if listener.exposure === 'network'}
                <span class="inline-flex items-center gap-1 rounded border border-blue-500/20 bg-blue-500/10 px-1.5 py-0.5 text-meta font-medium text-blue-400">
                  <Globe size={10} /> Network
                </span>
              {:else}
                <span class="inline-flex items-center gap-1 rounded bg-secondary px-1.5 py-0.5 text-meta font-medium text-muted-foreground">Loopback</span>
              {/if}
            </div>
            <span class="w-14 shrink-0 text-right font-mono text-meta text-muted-foreground">{formatProcessAge(listener.started_at)}</span>
            <div class="w-20 shrink-0 text-right">
              {#if listener.can_release}
                <Button
                  id={releaseButtonId(listener)}
                  variant="outline"
                  size="sm"
                  class="gap-1 text-xs opacity-80 hover:border-destructive/40 hover:text-destructive group-hover:opacity-100"
                  disabled={developmentPortsStore.releasingId !== null}
                  onclick={() => openReleaseDialog(listener)}
                  title={isWindows ? `Force release port ${listener.port}` : `Request graceful release of port ${listener.port}`}
                >
                  <LogOut size={12} /> Release
                </Button>
              {:else}
                <span class="inline-flex cursor-help items-center gap-1 rounded bg-secondary/40 px-2 py-1 text-meta font-medium text-muted-foreground/70" title={listener.blocked_reason || 'Protected process cannot be released'}>
                  <ShieldCheck size={11} class="opacity-70" /> Protected
                </span>
              {/if}
            </div>
          </div>
        </div>
      {/each}
    </div>
  {:else if searchQuery.trim()}
    <div class="space-y-2 rounded-xl border border-border/80 bg-card/70 p-8 text-center">
      <p class="text-xs text-muted-foreground">No development servers matching "{searchQuery}"</p>
      <Button variant="ghost" size="sm" onclick={() => (searchQuery = '')} class="text-xs">Clear Search</Button>
    </div>
  {:else}
    <EmptyState
      icon={Server}
      title="No supported development or testing tools are listening."
      description="Vite, Next.js, agent-browser, and other verified local listeners will appear here."
    />
  {/if}

  {#if pendingReleaseListener}
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-4 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="release-title">
      <Card class="w-full max-w-md space-y-4 border-border bg-card p-5 shadow-2xl">
        <div class="flex items-start gap-3">
          <div class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-warning/10 text-warning"><Radio size={17} /></div>
          <div>
            <h3 id="release-title" class="text-sm font-semibold">Release {pendingReleaseListener.server_name} on {pendingReleaseListener.bind_address}:{pendingReleaseListener.port}?</h3>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
              This will request graceful termination (SIGTERM) for PID {pendingReleaseListener.pid}{#if pendingReleaseListener.project_name} belonging to project <span class="font-semibold text-foreground">{pendingReleaseListener.project_name}</span>{/if}.
            </p>
          </div>
        </div>
        <div class="rounded-lg border border-border/70 bg-secondary/40 px-3 py-2.5 text-meta leading-relaxed text-muted-foreground">Active browser tabs, hot module reload sessions, or in-flight HTTP requests to this server will stop. Zenith will check if the port is freed.</div>
        <div class="flex justify-end gap-2 pt-1">
          <Button id="release-cancel" variant="ghost" size="sm" onclick={closeDialogs}>Cancel</Button>
          <Button variant="outline" size="sm" disabled={developmentPortsStore.releasingId !== null} onclick={handleReleaseNormally}>Release Normally</Button>
        </div>
      </Card>
    </div>
  {/if}

  {#if pendingForceListener}
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-4 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="force-release-title">
      <Card class="w-full max-w-md space-y-4 border-destructive/40 bg-card p-5 shadow-2xl">
        <div class="flex items-start gap-3">
          <div class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-destructive/10 text-destructive"><TriangleAlert size={17} /></div>
          <div>
            <h3 id="force-release-title" class="text-sm font-semibold text-destructive">Force Release Port {pendingForceListener.port}?</h3>
            <p class="mt-1 text-xs leading-relaxed text-muted-foreground">
              <span class="font-semibold text-foreground">{pendingForceListener.server_name}</span> (PID {pendingForceListener.pid})
              {isWindows ? ' can only be forcefully terminated on Windows.' : ' did not exit after the graceful stop request.'}
            </p>
          </div>
        </div>
        <div class="rounded-lg border border-destructive/20 bg-destructive/5 px-3 py-2.5 text-meta leading-relaxed text-destructive/90">Force release {isWindows ? 'terminates the process immediately' : 'sends SIGKILL immediately'}. Unsaved work or open database transactions in this server process may not shut down cleanly.</div>
        <div class="flex justify-end gap-2 pt-1">
          <Button id="force-release-cancel" variant="ghost" size="sm" onclick={closeDialogs}>Cancel</Button>
          <Button variant="destructive" size="sm" disabled={developmentPortsStore.releasingId !== null} onclick={handleForceRelease}>Force Release</Button>
        </div>
      </Card>
    </div>
  {/if}
</div>
