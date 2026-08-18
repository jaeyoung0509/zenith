<script lang="ts">
  import { onMount } from 'svelte';
  import { awakeStore } from '../../lib/stores/awake.svelte';
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import { formatDuration } from '../../lib/utils/format';
  import { tauriPickKeepAwakeApplication } from '../../lib/utils/tauri';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import {
    Moon,
    Sun,
    Clock,
    Plus,
    Check,
    Power,
    Shield,
    AlertCircle,
    AppWindow,
    FolderOpen,
  } from 'lucide-svelte';

  onMount(() => {
    awakeStore.refresh();
  });

  let awakeState = $derived(awakeStore.state);
  let rules = $derived(settingsStore.settings.awake_rules);

  let newAppName = $state('');
  let newExecutable = $state('');
  let showAddModal = $state(false);
  let selectedAppPath = $state('');
  let isPickingApp = $state(false);
  let pickerError = $state<string | null>(null);

  function handleSetTimer(mins: number | null) {
    if (mins === null) {
      awakeStore.setManual(null);
    } else {
      awakeStore.setManual(mins * 60);
    }
  }

  async function handleAddRule() {
    if (!newAppName.trim() || !newExecutable.trim()) return;
    await awakeStore.addRule({
      id: `rule.${Date.now()}`,
      app_name: newAppName.trim(),
      executable_pattern: newExecutable.trim(),
      behavior: 'prevent_system_sleep',
      enabled: true,
    });
    newAppName = '';
    newExecutable = '';
    selectedAppPath = '';
    showAddModal = false;
  }

  async function pickApplication() {
    if (isPickingApp) return;
    isPickingApp = true;
    pickerError = null;
    try {
      const selection = await tauriPickKeepAwakeApplication();
      if (selection) {
        newAppName = selection.name;
        newExecutable = selection.executable_pattern;
        selectedAppPath = selection.path;
      }
    } catch (error: any) {
      pickerError = error?.toString() || 'Could not open the application picker';
    } finally {
      isPickingApp = false;
    }
  }

  function closeAddModal() {
    showAddModal = false;
    pickerError = null;
  }
</script>

<div class="space-y-6">
  <!-- Header -->
  <div class="flex items-center justify-between pb-3 border-b border-border/60">
    <div class="flex items-center gap-3">
      <div class="h-9 w-9 rounded-lg bg-indigo-500/10 text-indigo-400 flex items-center justify-center">
        <Moon size={20} />
      </div>
      <div>
        <div class="flex items-center gap-2">
          <h2 class="text-base font-semibold text-foreground tracking-tight">Keep Awake Engine</h2>
          {#if awakeState.is_active}
            <Badge variant="warning" class="animate-pulse">Active</Badge>
          {:else}
            <Badge variant="secondary">Idle</Badge>
          {/if}
        </div>
        <p class="text-xs text-muted-foreground mt-0.5">
          Prevent idle sleep during long AI tasks, Docker builds, and developer script executions using macOS IOKit power assertions.
        </p>
      </div>
    </div>
  </div>

  <!-- Active Status Banner -->
  <Card class="p-5 {awakeState.is_active ? 'bg-amber-500/10 border-amber-500/30' : 'bg-card/60'} transition-colors">
    <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
      <div class="space-y-1">
        <div class="flex items-center gap-2">
          <Power size={16} class={awakeState.is_active ? 'text-amber-500' : 'text-muted-foreground'} />
          <h3 class="text-sm font-semibold text-foreground">
            {awakeState.is_active ? 'Power Assertion Active' : 'System Sleep Normal'}
          </h3>
        </div>
        <p class="text-xs text-muted-foreground">
          {#if awakeState.is_active}
            {awakeState.trigger_source || 'System sleep is currently prevented by Zenith.'}
            {#if awakeState.manual_expires_at}
              <span class="font-mono text-foreground ml-1">
                (Expires at {new Date(awakeState.manual_expires_at * 1000).toLocaleTimeString()})
              </span>
            {/if}
          {:else}
            Watching for configured background developer processes. Power assertion will automatically engage when active.
          {/if}
        </p>
      </div>

      {#if awakeState.is_active}
        <Button variant="destructive" size="sm" onclick={() => awakeStore.disableManual()}>
          Release Sleep Assertion
        </Button>
      {/if}
    </div>
  </Card>

  <!-- Quick Manual Timers -->
  <div class="space-y-3">
    <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
      Quick Manual Duration (Caffeinate)
    </h3>
    <div class="grid grid-cols-2 sm:grid-cols-4 gap-3">
      <Button
        variant="outline"
        size="md"
        onclick={() => handleSetTimer(30)}
        class="flex-col h-auto py-3 text-xs gap-1"
      >
        <Clock size={16} class="text-muted-foreground" />
        <span class="font-medium text-foreground">30 Minutes</span>
      </Button>

      <Button
        variant="outline"
        size="md"
        onclick={() => handleSetTimer(60)}
        class="flex-col h-auto py-3 text-xs gap-1"
      >
        <Clock size={16} class="text-muted-foreground" />
        <span class="font-medium text-foreground">1 Hour</span>
      </Button>

      <Button
        variant="outline"
        size="md"
        onclick={() => handleSetTimer(120)}
        class="flex-col h-auto py-3 text-xs gap-1"
      >
        <Clock size={16} class="text-muted-foreground" />
        <span class="font-medium text-foreground">2 Hours</span>
      </Button>

      <Button
        variant="outline"
        size="md"
        onclick={() => handleSetTimer(null)}
        class="flex-col h-auto py-3 text-xs gap-1"
      >
        <Sun size={16} class="text-amber-500" />
        <span class="font-medium text-foreground">Indefinite</span>
      </Button>
    </div>
  </div>

  <!-- App-Triggered Automation Rules -->
  <div class="space-y-3 pt-2">
    <div class="flex items-center justify-between">
      <div>
        <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
          Process-Triggered Rules
        </h3>
        <p class="text-[11px] text-muted-foreground">
          Automatically prevent sleep whenever these applications or CLI processes are running.
        </p>
      </div>

      <Button
        variant="outline"
        size="sm"
        onclick={() => (showAddModal = true)}
        class="gap-1.5 text-xs"
      >
        <Plus size={13} />
        <span>Add Rule</span>
      </Button>
    </div>

    <div class="space-y-2">
      {#each rules as rule (rule.id)}
        <div class="flex items-center justify-between p-3.5 rounded-xl border border-border/80 bg-card/70 text-xs">
          <div class="space-y-0.5">
            <div class="font-medium text-foreground">{rule.app_name}</div>
            <div class="text-[11px] text-muted-foreground font-mono">
              Pattern: {rule.executable_pattern} • {rule.behavior === 'prevent_system_sleep' ? 'Prevent System Sleep' : 'Keep Display Awake'}
            </div>
          </div>

          <label class="relative inline-flex items-center cursor-pointer">
            <input
              type="checkbox"
              checked={rule.enabled}
              onchange={() => awakeStore.toggleRule(rule.id)}
              class="sr-only peer"
            />
            <div
              class="w-9 h-5 bg-secondary peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-primary"
            ></div>
          </label>
        </div>
      {/each}
    </div>
  </div>

  <!-- Add Custom Rule Modal -->
  {#if showAddModal}
    <div class="fixed inset-0 z-50 bg-background/80 backdrop-blur-sm flex items-center justify-center p-4">
      <Card class="w-full max-w-md bg-card border-border shadow-2xl p-5 space-y-4">
        <h3 class="text-sm font-semibold text-foreground">Add Keep Awake App Rule</h3>

        <button
          type="button"
          onclick={pickApplication}
          disabled={isPickingApp}
          class="flex w-full items-center gap-3 rounded-xl border border-indigo-500/25 bg-indigo-500/5 p-3 text-left transition-colors hover:bg-indigo-500/10 disabled:opacity-50"
        >
          <div class="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-indigo-500/10 text-indigo-500">
            <AppWindow size={18} />
          </div>
          <div class="min-w-0 flex-1">
            <p class="text-xs font-medium text-foreground">{isPickingApp ? 'Opening Applications…' : 'Choose from Applications'}</p>
            <p class="mt-0.5 truncate text-[10px] text-muted-foreground">
              {selectedAppPath || 'Select a macOS app and fill its executable automatically.'}
            </p>
          </div>
          <FolderOpen size={15} class="shrink-0 text-muted-foreground" />
        </button>

        {#if pickerError}
          <p class="rounded-lg border border-red-500/20 bg-red-500/5 px-3 py-2 text-[11px] text-red-500">{pickerError}</p>
        {/if}

        <div class="flex items-center gap-3 text-[10px] uppercase tracking-wider text-muted-foreground">
          <span class="h-px flex-1 bg-border"></span>
          <span>or enter a CLI process</span>
          <span class="h-px flex-1 bg-border"></span>
        </div>

        <div class="space-y-3 text-xs">
          <div class="space-y-1">
            <label for="appName" class="text-muted-foreground font-medium">Application Name</label>
            <input
              id="appName"
              type="text"
              bind:value={newAppName}
              placeholder="e.g. ffmpeg render"
              class="w-full h-8 px-3 rounded-lg border border-border bg-background text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
            />
          </div>

          <div class="space-y-1">
            <label for="execPattern" class="text-muted-foreground font-medium">Executable Name Fragments</label>
            <input
              id="execPattern"
              type="text"
              bind:value={newExecutable}
              placeholder="e.g. ffmpeg|blender (separate with |)"
              class="w-full h-8 px-3 rounded-lg border border-border bg-background text-foreground focus:outline-none focus:ring-1 focus:ring-ring font-mono"
            />
          </div>
        </div>

        <div class="flex justify-end gap-2 pt-2">
          <Button variant="ghost" size="sm" onclick={closeAddModal}>
            Cancel
          </Button>
          <Button variant="primary" size="sm" onclick={handleAddRule}>
            Save Rule
          </Button>
        </div>
      </Card>
    </div>
  {/if}
</div>
