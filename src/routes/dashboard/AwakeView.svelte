<script lang="ts">
  import { onMount } from 'svelte';
  import { awakeStore } from '../../lib/stores/awake.svelte';
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import { formatDuration, formatTimeUntil } from '../../lib/utils/format';
  import { tauriPickKeepAwakeApplication } from '../../lib/utils/tauri';
  import type { AwakeBehavior, AwakeRule, PowerCondition } from '../../lib/models/types';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import Switch from '../../lib/components/Switch.svelte';
  import {
    Moon,
    Sun,
    Clock,
    Plus,
    Power,
    Shield,
    AlertCircle,
    AppWindow,
    FolderOpen,
    Zap,
    Battery,
    Monitor,
    Trash2,
  } from 'lucide-svelte';

  let now = $state(Date.now());

  onMount(() => {
    void awakeStore.refresh();

    // Local 1s timer for smooth countdown re-rendering without IPC
    const countdownTimer = setInterval(() => {
      now = Date.now();
    }, 1000);

    // 5s periodic refresh for lightweight AwakeState snapshot reading
    const stateTimer = setInterval(() => {
      void awakeStore.refresh();
    }, 5000);

    return () => {
      clearInterval(countdownTimer);
      clearInterval(stateTimer);
    };
  });

  let awakeState = $derived(awakeStore.state);
  let rules = $derived(settingsStore.settings.awake_rules);
  let activeRule = $derived(rules.find((r) => r.id === awakeState.active_rule_id) ?? null);

  // Manual timer behavior
  let manualBehavior = $state<AwakeBehavior>('prevent_system_sleep');

  // Add rule modal state
  let showAddModal = $state(false);
  let newAppName = $state('');
  let newExecutable = $state('');
  let newBehavior = $state<AwakeBehavior>('prevent_system_sleep');
  let newPowerCondition = $state<PowerCondition>('ac_power_only');
  let selectedAppPath = $state('');
  let isPickingApp = $state(false);
  let pickerError = $state<string | null>(null);

  function handleSetTimer(mins: number | null) {
    if (mins === null) {
      awakeStore.setManual(null, manualBehavior);
    } else {
      awakeStore.setManual(mins * 60, manualBehavior);
    }
  }

  async function handleAddRule() {
    if (!newAppName.trim() || !newExecutable.trim()) return;
    await awakeStore.addRule({
      id: `rule.${Date.now()}`,
      app_name: newAppName.trim(),
      executable_pattern: newExecutable.trim(),
      behavior: newBehavior,
      power_condition: newPowerCondition,
      enabled: true,
    });
    resetModal();
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

  function resetModal() {
    newAppName = '';
    newExecutable = '';
    newBehavior = 'prevent_system_sleep';
    newPowerCondition = 'ac_power_only';
    selectedAppPath = '';
    pickerError = null;
    showAddModal = false;
  }

  function getRuleEvaluation(ruleId: string) {
    return awakeState.rule_evaluations?.find((e) => e.rule_id === ruleId);
  }

  function formatCountdown(expiresAt: number) {
    // Reference `now` to guarantee reactive updates each second
    const diffSecs = Math.max(0, expiresAt - Math.floor(now / 1000));
    if (diffSecs <= 0) {
      return 'expiring now';
    }
    const timeUntil = formatTimeUntil(expiresAt);
    return timeUntil ? `${timeUntil} remaining` : `${diffSecs}s remaining`;
  }

  async function handleEnableRecommendedRules() {
    const updatedRules = rules.map((r) => {
      if (['rule.codex', 'rule.claude', 'rule.docker'].includes(r.id)) {
        return { ...r, enabled: true, power_condition: 'ac_power_only' as PowerCondition };
      }
      return r;
    });
    await settingsStore.save({ awake_rules: updatedRules });
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
            <Badge variant="warning" class="gap-1.5 font-medium">
              <span class="relative flex h-1.5 w-1.5">
                <span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-400 opacity-75"></span>
                <span class="relative inline-flex rounded-full h-1.5 w-1.5 bg-amber-500"></span>
              </span>
              <span>Active</span>
            </Badge>
          {:else}
            <Badge variant="secondary">Idle</Badge>
          {/if}
        </div>
        <p class="text-xs text-muted-foreground mt-0.5">
          Prevent idle sleep during long AI sessions, builds, and renders using macOS IOKit power assertions.
        </p>
      </div>
    </div>

    <!-- Power Source Badge -->
    <div class="flex items-center gap-2 px-2.5 py-1.5 rounded-lg border border-border/70 bg-card/60 text-xs">
      {#if awakeState.power_source === 'ac'}
        <Zap size={14} class="text-emerald-400" />
        <span class="font-medium text-foreground">Plugged In (AC)</span>
      {:else if awakeState.power_source === 'battery'}
        <Battery size={14} class="text-amber-400" />
        <span class="font-medium text-foreground">Battery Power</span>
      {:else}
        <Shield size={14} class="text-muted-foreground" />
        <span class="text-muted-foreground">Power: Unknown</span>
      {/if}
    </div>
  </div>

  <!-- Last Error Alert (if native assertion failed) -->
  {#if awakeState.last_error}
    <div class="flex items-start gap-2.5 rounded-xl border border-red-500/30 bg-red-500/10 p-4 text-xs text-red-400">
      <AlertCircle size={16} class="shrink-0 mt-0.5" />
      <div class="space-y-1">
        <div class="font-semibold">Native Power Assertion Error</div>
        <div class="text-[11px] leading-relaxed opacity-90">{awakeState.last_error}</div>
      </div>
    </div>
  {/if}

  <!-- Active Status Banner -->
  <Card class="p-5 {awakeState.is_active ? 'bg-amber-500/10 border-amber-500/30 shadow-sm' : 'bg-card/60'} transition-colors duration-200">
    <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
      <div class="space-y-1.5">
        <div class="flex items-center gap-2">
          <Power size={16} class={awakeState.is_active ? 'text-amber-500 animate-pulse-soft' : 'text-muted-foreground'} />
          <h3 class="text-sm font-semibold text-foreground">
            {#if awakeState.is_active}
              {#if awakeState.manual_expires_at != null || awakeState.trigger_source?.includes('Manual')}
                Keep Awake Active (Manual Session)
              {:else if activeRule}
                Keep Awake Active ({activeRule.app_name})
              {:else}
                Keep Awake Active
              {/if}
            {:else}
              System Sleep Normal
            {/if}
          </h3>
        </div>

        <p class="text-xs text-muted-foreground leading-relaxed">
          {#if awakeState.is_active}
            {#if awakeState.manual_expires_at != null}
              <span>Manual session • <strong class="text-foreground font-mono">{formatCountdown(awakeState.manual_expires_at)}</strong> (until {new Date(awakeState.manual_expires_at * 1000).toLocaleTimeString()}) • {awakeState.behavior === 'keep_display_awake' ? 'Display kept awake' : 'Mac awake (display may sleep)'}</span>
            {:else if awakeState.trigger_source?.includes('Manual')}
              <span>Manual indefinite session active • {awakeState.behavior === 'keep_display_awake' ? 'Display kept awake' : 'Mac awake (display may sleep)'}</span>
            {:else if activeRule}
              <span><strong class="text-foreground">{activeRule.app_name}</strong> is running • {activeRule.power_condition === 'ac_power_only' ? 'Plugged In (AC)' : 'Always'} • {activeRule.behavior === 'keep_display_awake' ? 'Display kept awake' : 'Mac awake (display may sleep)'}</span>
            {:else}
              <span>{awakeState.trigger_source || 'System sleep is currently prevented by Zenith.'}</span>
            {/if}
          {:else}
            <span>Watching {awakeState.active_rules_count} enabled process {awakeState.active_rules_count === 1 ? 'rule' : 'rules'}. Power assertion will engage automatically when matching processes run.</span>
          {/if}
        </p>
      </div>

      {#if awakeState.is_active}
        {#if awakeState.manual_expires_at != null || awakeState.trigger_source?.includes('Manual')}
          <Button variant="destructive" size="sm" onclick={() => awakeStore.disableManual()}>
            Stop Manual Session
          </Button>
        {:else if activeRule}
          <Button variant="outline" size="sm" class="text-xs border-amber-500/40 hover:bg-amber-500/20" onclick={() => awakeStore.toggleRule(activeRule.id)}>
            Disable {activeRule.app_name} Rule
          </Button>
        {:else}
          <Button variant="destructive" size="sm" onclick={() => awakeStore.disableManual()}>
            Release Sleep Assertion
          </Button>
        {/if}
      {/if}
    </div>
  </Card>

  <!-- Quick Manual Timers (Caffeinate) -->
  <div class="space-y-3">
    <div class="flex items-center justify-between">
      <div>
        <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
          Quick Manual Duration (Caffeinate)
        </h3>
        <p class="text-[11px] text-muted-foreground mt-0.5">
          Temporarily keep your Mac awake regardless of background rules.
        </p>
      </div>

      <!-- Mode Selector -->
      <div class="flex items-center gap-1.5 p-1 rounded-lg bg-card/80 border border-border/70 text-xs">
        <button
          type="button"
          onclick={() => (manualBehavior = 'prevent_system_sleep')}
          class="px-2.5 py-1 rounded text-[11px] font-medium transition-all {manualBehavior === 'prevent_system_sleep' ? 'bg-primary text-primary-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
          title="Work continues while you're away; display may turn off"
        >
          Keep Mac Awake
        </button>
        <button
          type="button"
          onclick={() => (manualBehavior = 'keep_display_awake')}
          class="px-2.5 py-1 rounded text-[11px] font-medium transition-all {manualBehavior === 'keep_display_awake' ? 'bg-primary text-primary-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
          title="Keeps both Mac and display awake while idle"
        >
          Keep Mac + Display Awake
        </button>
      </div>
    </div>

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

  <!-- Keep Awake Recommended Rules Onboarding (when all rules are disabled) -->
  {#if rules.length > 0 && rules.every((r) => !r.enabled)}
    <Card class="p-4 bg-indigo-500/5 border-indigo-500/20 space-y-3">
      <div class="flex items-start gap-3">
        <div class="h-8 w-8 rounded-lg bg-indigo-500/10 text-indigo-400 flex items-center justify-center shrink-0 mt-0.5">
          <Zap size={16} />
        </div>
        <div class="space-y-1 flex-1">
          <div class="text-xs font-semibold text-foreground">Enable Recommended Keep Awake Rules?</div>
          <p class="text-[11px] text-muted-foreground leading-relaxed">
            Zenith can automatically keep your Mac awake while you are actively working with development tools (Codex, Claude Code, Docker) and plugged into AC power. Rules stay disabled by default until you explicitly opt in.
          </p>
          <div class="pt-2">
            <Button variant="primary" size="sm" onclick={handleEnableRecommendedRules}>
              Enable Recommended Rules (AC Only)
            </Button>
          </div>
        </div>
      </div>
    </Card>
  {/if}

  <!-- Process-Triggered Automation Rules -->
  <div class="space-y-3 pt-2">
    <div class="flex items-center justify-between">
      <div>
        <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
          Process-Triggered Rules
        </h3>
        <p class="text-[11px] text-muted-foreground mt-0.5">
          Automatically prevent sleep while selected CLI processes or applications are running.
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

    {#if rules.length === 0}
      <Card class="p-8 text-center space-y-3 bg-card/40 border-dashed">
        <div class="h-10 w-10 rounded-full bg-indigo-500/10 text-indigo-400 flex items-center justify-center mx-auto">
          <Moon size={20} />
        </div>
        <div class="space-y-1">
          <h4 class="text-sm font-semibold text-foreground">Keep long-running work alive while you're away</h4>
          <p class="text-xs text-muted-foreground max-w-md mx-auto leading-relaxed">
            Zenith can watch Codex, Claude Code, long builds, 3D renders, and other processes and keep your Mac awake only when needed.
          </p>
        </div>
        <Button variant="primary" size="sm" onclick={() => (showAddModal = true)} class="gap-1.5">
          <Plus size={13} />
          Add Keep Awake Rule
        </Button>
      </Card>
    {:else}
      <div class="space-y-2">
        {#each rules as rule (rule.id)}
          {@const evaluation = getRuleEvaluation(rule.id)}
          <div class="flex items-center justify-between p-3.5 rounded-xl border border-border/80 bg-card/70 text-xs transition-all hover:bg-card/90">
            <div class="space-y-1 min-w-0 flex-1 pr-4">
              <div class="flex items-center gap-2">
                <span class="font-medium text-foreground truncate">{rule.app_name}</span>
                {#if evaluation}
                  {#if awakeState.active_rule_id === rule.id}
                    <span class="px-2 py-0.5 rounded-full text-[10px] font-semibold border border-emerald-500/30 bg-emerald-500/10 text-emerald-400">
                      Active Trigger
                    </span>
                  {:else if evaluation.status === 'active'}
                    <span class="px-2 py-0.5 rounded-full text-[10px] font-medium border border-emerald-500/20 bg-emerald-500/5 text-emerald-400/80" title="Process is running and eligible; ready to maintain sleep assertion if active trigger stops.">
                      Running (Standby)
                    </span>
                  {:else if evaluation.status === 'waiting_power'}
                    <span class="px-2 py-0.5 rounded-full text-[10px] font-medium border border-amber-500/30 bg-amber-500/10 text-amber-400" title="Process is running, but rule requires AC Power while Mac is on battery.">
                      Waiting for AC power
                    </span>
                  {:else if evaluation.status === 'waiting_process'}
                    <span class="px-2 py-0.5 rounded-full text-[10px] font-medium border border-border bg-secondary/50 text-muted-foreground">
                      Waiting for process
                    </span>
                  {:else}
                    <span class="px-2 py-0.5 rounded-full text-[10px] font-medium border border-border/50 text-muted-foreground/60">
                      Disabled
                    </span>
                  {/if}
                {/if}
              </div>

              <div class="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[11px] text-muted-foreground">
                <span class="font-mono text-[10px] bg-secondary/60 px-1.5 py-0.5 rounded border border-border/40">
                  {rule.executable_pattern}
                </span>
                <span>•</span>
                <span>{rule.power_condition === 'always' ? 'Always (AC & Battery)' : 'Plugged In (AC) Only'}</span>
                <span>•</span>
                <span>{rule.behavior === 'prevent_system_sleep' ? 'Keep Mac awake' : 'Keep Mac + display awake'}</span>
              </div>
            </div>

            <div class="flex items-center gap-3 shrink-0">
              <button
                type="button"
                onclick={() => awakeStore.deleteRule(rule.id)}
                class="p-1.5 rounded-lg text-muted-foreground hover:text-red-400 hover:bg-red-500/10 transition-colors"
                title="Delete rule"
                aria-label={`Delete ${rule.app_name} rule`}
              >
                <Trash2 size={14} />
              </button>

              <Switch
                checked={rule.enabled}
                onchange={() => awakeStore.toggleRule(rule.id)}
                ariaLabel={`Toggle ${rule.app_name} rule`}
              />
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <!-- Add Custom Rule Modal -->
  {#if showAddModal}
    <div class="fixed inset-0 z-50 bg-background/80 backdrop-blur-sm flex items-center justify-center p-4">
      <Card class="w-full max-w-lg bg-card border-border shadow-2xl p-5 space-y-4">
        <div class="flex items-center justify-between border-b border-border/60 pb-3">
          <h3 class="text-sm font-semibold text-foreground">Add Keep Awake Rule</h3>
          <button type="button" onclick={resetModal} class="text-muted-foreground hover:text-foreground text-xs">✕</button>
        </div>

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
              {selectedAppPath || 'Select an installed macOS app to auto-fill its executable pattern.'}
            </p>
          </div>
          <FolderOpen size={15} class="shrink-0 text-muted-foreground" />
        </button>

        {#if pickerError}
          <p class="rounded-lg border border-red-500/20 bg-red-500/5 px-3 py-2 text-[11px] text-red-500">{pickerError}</p>
        {/if}

        <div class="flex items-center gap-3 text-[10px] uppercase tracking-wider text-muted-foreground">
          <span class="h-px flex-1 bg-border"></span>
          <span>or enter manual configuration</span>
          <span class="h-px flex-1 bg-border"></span>
        </div>

        <div class="space-y-3.5 text-xs">
          <div class="space-y-1">
            <label for="appName" class="text-muted-foreground font-medium">Application or Process Name</label>
            <input
              id="appName"
              type="text"
              bind:value={newAppName}
              placeholder="e.g. Codex / ffmpeg / Blender"
              class="w-full h-8 px-3 rounded-lg border border-border bg-background text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
            />
          </div>

          <div class="space-y-1">
            <label for="execPattern" class="text-muted-foreground font-medium">Executable Name Fragments</label>
            <input
              id="execPattern"
              type="text"
              bind:value={newExecutable}
              placeholder="e.g. codex|ffmpeg|blender (separate with |)"
              class="w-full h-8 px-3 rounded-lg border border-border bg-background text-foreground focus:outline-none focus:ring-1 focus:ring-ring font-mono"
            />
            <p class="text-[10px] text-muted-foreground">Matches any running process whose executable name contains these words.</p>
          </div>

          <!-- Power Condition Selection -->
          <div class="space-y-1.5 pt-1">
            <span class="text-muted-foreground font-medium">Power Condition</span>
            <div class="grid grid-cols-2 gap-2">
              <button
                type="button"
                onclick={() => (newPowerCondition = 'ac_power_only')}
                class="p-2.5 rounded-lg border text-left transition-all {newPowerCondition === 'ac_power_only' ? 'border-primary bg-primary/5 text-foreground' : 'border-border/70 text-muted-foreground hover:bg-secondary/40'}"
              >
                <div class="font-medium flex items-center gap-1.5">
                  <Zap size={13} class="text-emerald-400" />
                  <span>Plugged In (AC) Only</span>
                </div>
                <div class="text-[10px] text-muted-foreground mt-0.5">Recommended. Saves battery life when unplugged.</div>
              </button>

              <button
                type="button"
                onclick={() => (newPowerCondition = 'always')}
                class="p-2.5 rounded-lg border text-left transition-all {newPowerCondition === 'always' ? 'border-primary bg-primary/5 text-foreground' : 'border-border/70 text-muted-foreground hover:bg-secondary/40'}"
              >
                <div class="font-medium flex items-center gap-1.5">
                  <Battery size={13} class="text-amber-400" />
                  <span>Always (AC or Battery)</span>
                </div>
                <div class="text-[10px] text-muted-foreground mt-0.5">Prevents sleep even when running on battery.</div>
              </button>
            </div>
          </div>

          <!-- Behavior Selection -->
          <div class="space-y-1.5 pt-1">
            <span class="text-muted-foreground font-medium">Sleep Behavior</span>
            <div class="grid grid-cols-2 gap-2">
              <button
                type="button"
                onclick={() => (newBehavior = 'prevent_system_sleep')}
                class="p-2.5 rounded-lg border text-left transition-all {newBehavior === 'prevent_system_sleep' ? 'border-primary bg-primary/5 text-foreground' : 'border-border/70 text-muted-foreground hover:bg-secondary/40'}"
              >
                <div class="font-medium flex items-center gap-1.5">
                  <Moon size={13} class="text-indigo-400" />
                  <span>Keep Mac Awake</span>
                </div>
                <div class="text-[10px] text-muted-foreground mt-0.5">Work continues while you're away. The display may turn off.</div>
              </button>

              <button
                type="button"
                onclick={() => (newBehavior = 'keep_display_awake')}
                class="p-2.5 rounded-lg border text-left transition-all {newBehavior === 'keep_display_awake' ? 'border-primary bg-primary/5 text-foreground' : 'border-border/70 text-muted-foreground hover:bg-secondary/40'}"
              >
                <div class="font-medium flex items-center gap-1.5">
                  <Monitor size={13} class="text-blue-400" />
                  <span>Keep Mac + Display Awake</span>
                </div>
                <div class="text-[10px] text-muted-foreground mt-0.5">Work continues and the display stays on while idle.</div>
              </button>
            </div>
          </div>
        </div>

        <div class="flex justify-end gap-2 pt-3 border-t border-border/60">
          <Button variant="ghost" size="sm" onclick={resetModal}>
            Cancel
          </Button>
          <Button variant="primary" size="sm" onclick={handleAddRule} disabled={!newAppName.trim() || !newExecutable.trim()}>
            Save Rule
          </Button>
        </div>
      </Card>
    </div>
  {/if}
</div>
