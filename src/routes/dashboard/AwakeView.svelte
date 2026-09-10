<script lang="ts">
  import { onMount } from 'svelte';
  import { awakeStore } from '../../lib/stores/awake.svelte';
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import { formatTimeUntil } from '../../lib/utils/format';
  import { tauriPickKeepAwakeApplication } from '../../lib/utils/tauri';
  import {
    AWAKE_AGENT_OPTIONS,
    awakeAgentLabel,
    awakeRuleSummary,
    selectedAwakeAgents,
    toggleAwakeAgent,
    typedAwakeRule,
  } from '../../lib/utils/awake';
  import type {
    ApplicationIdentity,
    AwakeAgentId,
    AwakeBehavior,
    AwakeRule,
    AwakeRuleStatus,
    PowerCondition,
  } from '../../lib/models/types';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import Switch from '../../lib/components/Switch.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import InlineNotice from '../../lib/components/InlineNotice.svelte';
  import {
    AlertCircle,
    AppWindow,
    Battery,
    Check,
    Clock,
    FolderOpen,
    Moon,
    Pencil,
    Plus,
    Power,
    Shield,
    Trash2,
    X,
    Zap,
  } from 'lucide-svelte';

  type EditorMode = 'basic' | 'advanced';

  let now = $state(Date.now());
  let showRuleEditor = $state(false);
  let editorMode = $state<EditorMode>('basic');
  let editingRuleId = $state<string | null>(null);
  let newApplication = $state<ApplicationIdentity | null>(null);
  let selectedAgents = $state<AwakeAgentId[]>([]);
  let newAppName = $state('');
  let newExecutable = $state('');
  let newRequiresPattern = $state('');
  let newBehavior = $state<AwakeBehavior>('prevent_system_sleep');
  let newPowerCondition = $state<PowerCondition>('ac_power_only');
  let isPickingApp = $state(false);
  let isSavingRule = $state(false);
  let pickerError = $state<string | null>(null);

  onMount(() => {
    void awakeStore.refresh();

    // Countdown is local only. The backend watcher remains event-driven and
    // evaluates at its existing bounded interval only while work is enabled.
    const countdownTimer = setInterval(() => {
      now = Date.now();
    }, 1000);

    // get_awake_state is an in-memory read; no process scan or power query is
    // performed by this UI refresh.
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
  let typedRules = $derived(rules.filter(typedAwakeRule));
  let legacyRules = $derived(rules.filter((rule) => !typedAwakeRule(rule)));
  let activeRule = $derived(rules.find((rule) => rule.id === awakeState.active_rule_id) ?? null);
  let editingRule = $derived(
    editingRuleId ? rules.find((rule) => rule.id === editingRuleId) ?? null : null
  );

  // Manual timer behavior is intentionally separate from automatic rules.
  let manualBehavior = $state<AwakeBehavior>('prevent_system_sleep');

  function handleSetTimer(minutes: number | null) {
    void awakeStore.setManual(minutes === null ? null : minutes * 60, manualBehavior);
  }

  function openBasicEditor(rule?: AwakeRule) {
    editorMode = 'basic';
    editingRuleId = rule?.id ?? null;
    newApplication = rule?.application ?? null;
    selectedAgents = rule ? selectedAwakeAgents(rule) : [];
    newAppName = rule?.app_name ?? '';
    newExecutable = rule?.executable_pattern ?? '';
    newRequiresPattern = rule?.requires_process_pattern ?? '';
    newBehavior = rule?.behavior ?? 'prevent_system_sleep';
    newPowerCondition = rule?.power_condition ?? 'ac_power_only';
    pickerError = null;
    showRuleEditor = true;
  }

  function openAdvancedEditor(rule?: AwakeRule) {
    editorMode = 'advanced';
    editingRuleId = rule?.id ?? null;
    newApplication = null;
    selectedAgents = [];
    newAppName = rule?.app_name ?? '';
    newExecutable = rule?.executable_pattern ?? '';
    newRequiresPattern = rule?.requires_process_pattern ?? '';
    newBehavior = rule?.behavior ?? 'prevent_system_sleep';
    newPowerCondition = rule?.power_condition ?? 'ac_power_only';
    pickerError = null;
    showRuleEditor = true;
  }

  function closeRuleEditor() {
    showRuleEditor = false;
    editingRuleId = null;
    newApplication = null;
    selectedAgents = [];
    newAppName = '';
    newExecutable = '';
    newRequiresPattern = '';
    newBehavior = 'prevent_system_sleep';
    newPowerCondition = 'ac_power_only';
    pickerError = null;
  }

  async function pickApplication() {
    if (isPickingApp) return;
    isPickingApp = true;
    pickerError = null;
    try {
      const selection = await tauriPickKeepAwakeApplication();
      if (!selection) return;

      if (editorMode === 'basic') {
        newApplication = {
          display_name: selection.name,
          executable_name: selection.executable_pattern,
          path: selection.path,
        };
      } else {
        // Advanced rules remain explicitly raw. The picker only helps fill the
        // primary legacy fragment and never silently upgrades the rule shape.
        newAppName = selection.name;
        newExecutable = selection.executable_pattern;
      }
    } catch (error: unknown) {
      pickerError = error instanceof Error ? error.message : String(error);
    } finally {
      isPickingApp = false;
    }
  }

  function removeAgent(agentId: AwakeAgentId) {
    selectedAgents = selectedAgents.filter((candidate) => candidate !== agentId);
  }

  function isEditorValid(): boolean {
    if (editorMode === 'basic') return newApplication !== null;
    return Boolean(newAppName.trim() && newExecutable.trim());
  }

  async function saveRule() {
    if (isSavingRule || !isEditorValid()) return;
    isSavingRule = true;
    pickerError = null;
    const existing = editingRule;
    const id = editingRuleId ?? `rule.${Date.now()}`;
    const enabled = existing?.enabled ?? false;
    const rule: AwakeRule = editorMode === 'basic'
      ? {
          id,
          app_name: newApplication?.display_name.trim() ?? '',
          executable_pattern: newApplication?.executable_name.trim() ?? '',
          requires_process_pattern: null,
          application: newApplication,
          agent_ids: [...selectedAgents],
          behavior: newBehavior,
          power_condition: newPowerCondition,
          enabled,
        }
      : {
          id,
          app_name: newAppName.trim(),
          executable_pattern: newExecutable.trim(),
          requires_process_pattern: newRequiresPattern.trim() || null,
          application: null,
          agent_ids: [],
          behavior: newBehavior,
          power_condition: newPowerCondition,
          enabled,
        };

    try {
      const saved = editingRuleId
        ? await awakeStore.updateRule(rule)
        : await awakeStore.addRule(rule);
      if (!saved) {
        pickerError = settingsStore.error ?? 'Could not save the rule. Please try again.';
        return;
      }
      closeRuleEditor();
    } catch (error: unknown) {
      pickerError = error instanceof Error ? error.message : String(error);
    } finally {
      isSavingRule = false;
    }
  }

  function getRuleEvaluation(ruleId: string) {
    return awakeState.rule_evaluations?.find((evaluation) => evaluation.rule_id === ruleId);
  }

  function formatCountdown(expiresAt: number) {
    const diffSeconds = Math.max(0, expiresAt - Math.floor(now / 1000));
    if (diffSeconds <= 0) return 'expiring now';
    const timeUntil = formatTimeUntil(expiresAt);
    return timeUntil ? `${timeUntil} remaining` : `${diffSeconds}s remaining`;
  }

  function statusLabel(status: AwakeRuleStatus): string {
    switch (status) {
      case 'active': return 'Ready';
      case 'waiting_application': return 'Waiting for app';
      case 'waiting_agent': return 'Waiting for an agent';
      case 'waiting_power': return 'Waiting for AC power';
      case 'invalid_application': return 'Application unavailable';
      case 'waiting_process': return 'Waiting for process';
      case 'disabled': return 'Disabled';
    }
  }

  function statusClass(status: AwakeRuleStatus): string {
    if (status === 'active') return 'border-success/25 bg-success/10 text-success';
    if (status === 'waiting_power') return 'border-warning/30 bg-warning/10 text-warning';
    if (status === 'invalid_application') return 'border-destructive/25 bg-destructive/10 text-destructive';
    if (status === 'disabled') return 'border-border/50 text-muted-foreground/70';
    return 'border-border bg-secondary/50 text-muted-foreground';
  }

  async function handleEnableRecommendedRules() {
    const updatedRules = rules.map((rule) => {
      if (['rule.codex', 'rule.claude', 'rule.docker'].includes(rule.id)) {
        return { ...rule, enabled: true, power_condition: 'ac_power_only' as PowerCondition };
      }
      return rule;
    });
    await settingsStore.save({ awake_rules: updatedRules });
  }
</script>

<div class="space-y-6">
  <PageHeader
    title="Keep Awake Engine"
    subtitle="Keep long-running work alive with explicit app, agent, power, and display choices."
    icon={Moon}
  >
    {#snippet badge()}
      {#if awakeState.is_active}
        <Badge variant="warning" class="gap-1.5 font-medium">
          <span class="relative flex h-1.5 w-1.5" aria-hidden="true">
            <span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-warning opacity-75"></span>
            <span class="relative inline-flex rounded-full h-1.5 w-1.5 bg-warning"></span>
          </span>
          Active
        </Badge>
      {:else}
        <Badge variant="secondary">Idle</Badge>
      {/if}
    {/snippet}

    {#snippet actions()}
      <div class="flex items-center gap-2 px-2.5 py-1.5 rounded-lg border border-border/70 bg-card/60 text-xs" aria-label="Current power source">
        {#if awakeState.power_source === 'ac'}
          <Zap size={14} class="text-success" aria-hidden="true" />
          <span class="font-medium text-foreground">Plugged In (AC)</span>
        {:else if awakeState.power_source === 'battery'}
          <Battery size={14} class="text-warning" aria-hidden="true" />
          <span class="font-medium text-foreground">Battery Power</span>
        {:else}
          <Shield size={14} class="text-muted-foreground" aria-hidden="true" />
          <span class="text-muted-foreground">Power: Unknown</span>
        {/if}
      </div>
    {/snippet}
  </PageHeader>

  {#if awakeState.last_error}
    <InlineNotice
      variant="destructive"
      title="Native Power Assertion Error"
      message={awakeState.last_error}
    />
  {/if}

  {#if settingsStore.error}
    <InlineNotice
      variant="destructive"
      title="Settings Save Error"
      message={settingsStore.error}
    />
  {/if}

  <Card class="p-5 {awakeState.is_active ? 'bg-warning/10 border-warning/30 shadow-sm' : 'bg-card/60'} transition-colors duration-200">
    <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
      <div class="space-y-1.5">
        <div class="flex items-center gap-2">
          <Power size={16} class={awakeState.is_active ? 'text-warning' : 'text-muted-foreground'} aria-hidden="true" />
          <h2 class="text-sm font-semibold text-foreground">
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
          </h2>
        </div>

        <p class="text-xs text-muted-foreground leading-relaxed">
          {#if awakeState.is_active}
            {#if awakeState.manual_expires_at != null}
              Manual session · <strong class="text-foreground font-mono">{formatCountdown(awakeState.manual_expires_at)}</strong> · {awakeState.behavior === 'keep_display_awake' ? 'Display stays awake.' : 'Display may sleep.'}
            {:else if awakeState.trigger_source?.includes('Manual')}
              Manual indefinite session active · {awakeState.behavior === 'keep_display_awake' ? 'Display stays awake.' : 'Display may sleep.'}
            {:else if activeRule}
              {awakeRuleSummary(activeRule)}
            {:else}
              {awakeState.trigger_source || 'System sleep is currently prevented by Zenith.'}
            {/if}
          {:else}
            Watching {awakeState.active_rules_count} enabled {awakeState.active_rules_count === 1 ? 'automatic rule' : 'automatic rules'}. Manual Keep Awake always takes priority.
          {/if}
        </p>
      </div>

      {#if awakeState.is_active}
        {#if awakeState.manual_expires_at != null || awakeState.trigger_source?.includes('Manual')}
          <Button variant="destructive" size="sm" onclick={() => void awakeStore.disableManual()}>
            Stop Manual Session
          </Button>
        {:else if activeRule}
          <Button variant="outline" size="sm" class="text-xs border-warning/40 hover:bg-warning/20" onclick={() => void awakeStore.toggleRule(activeRule.id)}>
            Disable {activeRule.app_name} Rule
          </Button>
        {/if}
      {/if}
    </div>
  </Card>

  <section class="space-y-3" aria-labelledby="manual-heading">
    <div class="flex flex-col gap-3 xl:flex-row xl:items-center xl:justify-between">
      <div>
        <h2 id="manual-heading" class="text-sm font-semibold tracking-tight text-foreground">Quick Manual Duration</h2>
        <p class="text-meta text-muted-foreground mt-0.5">Temporarily keep the computer awake regardless of automatic rules.</p>
      </div>
      <div class="flex items-center gap-1.5 p-1 rounded-lg bg-card/80 border border-border/70 text-xs" role="group" aria-label="Manual sleep behavior">
        <button
          type="button"
          aria-pressed={manualBehavior === 'prevent_system_sleep'}
          onclick={() => (manualBehavior = 'prevent_system_sleep')}
          class="px-2.5 py-1 rounded text-meta font-medium transition-colors {manualBehavior === 'prevent_system_sleep' ? 'bg-primary text-primary-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
          title="Work continues while you are away; the display may turn off"
        >
          Computer awake
        </button>
        <button
          type="button"
          aria-pressed={manualBehavior === 'keep_display_awake'}
          onclick={() => (manualBehavior = 'keep_display_awake')}
          class="px-2.5 py-1 rounded text-meta font-medium transition-colors {manualBehavior === 'keep_display_awake' ? 'bg-primary text-primary-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
          title="Keeps both the computer and display awake while idle"
        >
          Computer + display
        </button>
      </div>
    </div>

    <div class="grid grid-cols-2 sm:grid-cols-4 gap-3">
      <Button variant="outline" size="md" onclick={() => handleSetTimer(30)} class="flex-col h-auto py-3 text-xs gap-1">
        <Clock size={16} class="text-muted-foreground" aria-hidden="true" />
        <span class="font-medium text-foreground">30 Minutes</span>
      </Button>
      <Button variant="outline" size="md" onclick={() => handleSetTimer(60)} class="flex-col h-auto py-3 text-xs gap-1">
        <Clock size={16} class="text-muted-foreground" aria-hidden="true" />
        <span class="font-medium text-foreground">1 Hour</span>
      </Button>
      <Button variant="outline" size="md" onclick={() => handleSetTimer(120)} class="flex-col h-auto py-3 text-xs gap-1">
        <Clock size={16} class="text-muted-foreground" aria-hidden="true" />
        <span class="font-medium text-foreground">2 Hours</span>
      </Button>
      <Button variant="outline" size="md" onclick={() => handleSetTimer(null)} class="flex-col h-auto py-3 text-xs gap-1">
        <Moon size={16} class="text-warning" aria-hidden="true" />
        <span class="font-medium text-foreground">Indefinite</span>
      </Button>
    </div>
  </section>

  {#if rules.length > 0 && rules.every((rule) => !rule.enabled)}
    <Card class="p-4 bg-ai/5 border-ai/20 space-y-3">
      <div class="flex items-start gap-3">
        <div class="h-8 w-8 rounded-lg bg-ai/10 text-ai flex items-center justify-center shrink-0 mt-0.5">
          <AlertCircle size={16} aria-hidden="true" />
        </div>
        <div class="space-y-1 flex-1">
          <div class="text-xs font-semibold text-foreground">Automatic rules are off</div>
          <p class="text-meta text-muted-foreground leading-relaxed">
            Rules are saved disabled to protect battery life. Review the rule summary, then enable only the rules you explicitly want to run.
          </p>
          <div class="pt-2">
            <Button variant="primary" size="sm" onclick={() => void handleEnableRecommendedRules()}>
              Enable recommended AC-only rules
            </Button>
          </div>
        </div>
      </div>
    </Card>
  {/if}

  <section class="space-y-3 pt-2" aria-labelledby="automatic-heading">
    <div class="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
      <div>
        <h2 id="automatic-heading" class="text-sm font-semibold tracking-tight text-foreground">Automatic app rules</h2>
        <p class="text-meta text-muted-foreground mt-0.5">Build an app-and-agent condition in plain language. Selected agents use OR semantics.</p>
      </div>
      <Button variant="outline" size="sm" onclick={() => openBasicEditor()} class="gap-1.5">
        <Plus size={13} aria-hidden="true" />
        Add app rule
      </Button>
    </div>

    {#if typedRules.length === 0}
      <Card class="p-7 text-center space-y-3 bg-card/40 border-dashed">
        <div class="h-10 w-10 rounded-full bg-ai/10 text-ai flex items-center justify-center mx-auto">
          <AppWindow size={20} aria-hidden="true" />
        </div>
        <div class="space-y-1">
          <h3 class="text-sm font-semibold text-foreground">Choose an installed app to get started</h3>
          <p class="text-xs text-muted-foreground max-w-lg mx-auto leading-relaxed">
            For example, choose Warp, select Codex or another agent, and keep the computer awake only while plugged in. Leaving agents empty creates an app-only rule.
          </p>
        </div>
        <Button variant="primary" size="sm" onclick={() => openBasicEditor()} class="gap-1.5">
          <Plus size={13} aria-hidden="true" />
          Build an app rule
        </Button>
      </Card>
    {:else}
      <div class="space-y-2">
        {#each typedRules as rule (rule.id)}
          {@const evaluation = getRuleEvaluation(rule.id)}
          <Card class="p-3.5 bg-card/70 border-border/80">
            <div class="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
              <div class="min-w-0 flex-1 space-y-2">
                <div class="flex flex-wrap items-center gap-2">
                  <span class="font-medium text-foreground truncate">{rule.application?.display_name ?? rule.app_name}</span>
                  {#if evaluation}
                    <span class="px-2 py-0.5 rounded-full text-caption font-medium border {statusClass(evaluation.status)}" title={evaluation.status === 'waiting_agent' ? 'The selected app is running, but none of the selected agents is running.' : undefined}>
                      {statusLabel(evaluation.status)}
                    </span>
                  {/if}
                  {#if !rule.enabled}
                    <Badge variant="secondary">Off</Badge>
                  {/if}
                </div>
                <p class="text-xs text-muted-foreground leading-relaxed">{awakeRuleSummary(rule)}</p>
                <div class="flex flex-wrap items-center gap-1.5 text-caption text-muted-foreground">
                  {#if (rule.agent_ids ?? []).length > 0}
                    <span>Any selected agent:</span>
                    {#each rule.agent_ids ?? [] as agentId}
                      <span class="inline-flex items-center rounded-full border border-ai/20 bg-ai/10 px-2 py-0.5 text-ai">{awakeAgentLabel(agentId)}</span>
                    {/each}
                  {:else}
                    <span>App alone triggers this rule</span>
                  {/if}
                  <span class="mx-1" aria-hidden="true">·</span>
                  <span>{rule.power_condition === 'ac_power_only' ? 'Plugged in only' : 'AC or battery'}</span>
                  <span class="mx-1" aria-hidden="true">·</span>
                  <span>{rule.behavior === 'keep_display_awake' ? 'Display stays awake' : 'Display may sleep'}</span>
                </div>
                {#if rule.application?.path}
                  <p class="truncate text-caption text-muted-foreground/80" title={rule.application.path}>Selected path: {rule.application.path}</p>
                {/if}
              </div>

              <div class="flex items-center gap-2 shrink-0">
                <button type="button" onclick={() => openBasicEditor(rule)} class="p-1.5 rounded-lg text-muted-foreground hover:text-foreground hover:bg-secondary/60 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" title={`Edit ${rule.app_name} rule`} aria-label={`Edit ${rule.app_name} rule`}>
                  <Pencil size={14} aria-hidden="true" />
                </button>
                <button type="button" onclick={() => void awakeStore.deleteRule(rule.id)} class="p-1.5 rounded-lg text-muted-foreground hover:text-destructive hover:bg-destructive/10 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" title={`Delete ${rule.app_name} rule`} aria-label={`Delete ${rule.app_name} rule`}>
                  <Trash2 size={14} aria-hidden="true" />
                </button>
                <Switch checked={rule.enabled} onchange={() => void awakeStore.toggleRule(rule.id)} ariaLabel={`Toggle ${rule.app_name} rule`} />
              </div>
            </div>
          </Card>
        {/each}
      </div>
    {/if}
  </section>

  <details class="group rounded-xl border border-border/70 bg-card/30">
    <summary class="flex cursor-pointer list-none items-center justify-between gap-3 p-4 text-xs font-semibold text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
      <span>Advanced / legacy rules ({legacyRules.length})</span>
      <span class="text-meta text-muted-foreground group-open:rotate-180 transition-transform" aria-hidden="true">⌄</span>
    </summary>
    <div class="border-t border-border/60 p-4 space-y-3">
      <p class="text-meta text-muted-foreground">
        Older custom rules are preserved losslessly. Their raw process fragments are evaluated only here and never mixed into the basic app builder.
      </p>
      <Button variant="outline" size="sm" onclick={() => openAdvancedEditor()} class="gap-1.5">
        <Plus size={13} aria-hidden="true" />
        Add custom legacy rule
      </Button>
      {#if legacyRules.length === 0}
        <p class="text-xs text-muted-foreground">No legacy rules are configured.</p>
      {:else}
        <div class="space-y-2">
          {#each legacyRules as rule (rule.id)}
            {@const evaluation = getRuleEvaluation(rule.id)}
            <div class="flex flex-col gap-3 rounded-lg border border-border/70 bg-card/60 p-3 sm:flex-row sm:items-start sm:justify-between">
              <div class="min-w-0 space-y-1.5">
                <div class="flex flex-wrap items-center gap-2">
                  <span class="text-xs font-medium text-foreground">{rule.app_name}</span>
                  {#if evaluation}
                    <span class="px-2 py-0.5 rounded-full text-caption font-medium border {statusClass(evaluation.status)}">{statusLabel(evaluation.status)}</span>
                  {/if}
                  {#if !rule.enabled}<Badge variant="secondary">Off</Badge>{/if}
                </div>
                <p class="font-mono text-caption text-muted-foreground break-all">Primary: {rule.executable_pattern}</p>
                {#if rule.requires_process_pattern}
                  <p class="font-mono text-caption text-muted-foreground break-all">Also require: {rule.requires_process_pattern}</p>
                {/if}
              </div>
              <div class="flex items-center gap-2 shrink-0">
                <button type="button" onclick={() => openAdvancedEditor(rule)} class="p-1.5 rounded-lg text-muted-foreground hover:text-foreground hover:bg-secondary/60 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" title={`Edit ${rule.app_name} legacy rule`} aria-label={`Edit ${rule.app_name} legacy rule`}>
                  <Pencil size={14} aria-hidden="true" />
                </button>
                <button type="button" onclick={() => void awakeStore.deleteRule(rule.id)} class="p-1.5 rounded-lg text-muted-foreground hover:text-destructive hover:bg-destructive/10 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" title={`Delete ${rule.app_name} legacy rule`} aria-label={`Delete ${rule.app_name} legacy rule`}>
                  <Trash2 size={14} aria-hidden="true" />
                </button>
                <Switch checked={rule.enabled} onchange={() => void awakeStore.toggleRule(rule.id)} ariaLabel={`Toggle ${rule.app_name} legacy rule`} />
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </details>

  {#if showRuleEditor}
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-background/80 p-4 backdrop-blur-sm" role="presentation">
      <div class="w-full max-w-xl max-h-[calc(100vh-2rem)] overflow-y-auto rounded-xl border border-border bg-card p-5 text-foreground shadow-2xl" role="dialog" aria-modal="true" aria-labelledby="rule-editor-title" tabindex="-1">
        <div class="flex items-start justify-between gap-3 border-b border-border/60 pb-3">
          <div>
            <h2 id="rule-editor-title" class="text-sm font-semibold">{editorMode === 'basic' ? 'Build an app rule' : 'Edit legacy process rule'}</h2>
            <p class="mt-1 text-meta text-muted-foreground">{editorMode === 'basic' ? 'Choose what must be present before Zenith holds a power assertion.' : 'Use raw process fragments only for custom or older rules.'}</p>
          </div>
          <button type="button" onclick={closeRuleEditor} disabled={isSavingRule} class="h-8 w-8 rounded-lg text-muted-foreground hover:bg-secondary hover:text-foreground disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" title="Close rule editor" aria-label="Close rule editor">
            <X size={16} class="mx-auto" aria-hidden="true" />
          </button>
        </div>

        {#if editorMode === 'basic'}
          <div class="space-y-5 pt-5">
            <fieldset class="space-y-2">
              <legend class="text-xs font-semibold text-foreground">When this application is open</legend>
              <p class="text-meta text-muted-foreground">Choose a native application. The saved identity includes its exact bundle or executable path.</p>
              <button type="button" onclick={() => void pickApplication()} disabled={isPickingApp} class="flex w-full items-center gap-3 rounded-xl border border-ai/25 bg-ai/5 p-3 text-left transition-colors hover:bg-ai/10 disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                <div class="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-ai/10 text-ai"><AppWindow size={18} aria-hidden="true" /></div>
                <div class="min-w-0 flex-1">
                  <p class="text-xs font-medium text-foreground">{isPickingApp ? 'Opening Applications…' : newApplication ? 'Change selected application' : 'Choose from installed applications'}</p>
                  <p class="mt-0.5 truncate text-caption text-muted-foreground">{newApplication ? `${newApplication.display_name} · ${newApplication.executable_name}` : 'Cancellation leaves the rule unchanged.'}</p>
                </div>
                <FolderOpen size={15} class="shrink-0 text-muted-foreground" aria-hidden="true" />
              </button>
              {#if newApplication}
                <div class="rounded-lg border border-border/70 bg-secondary/30 px-3 py-2 text-caption text-muted-foreground">
                  <div class="flex items-center justify-between gap-3"><span class="font-medium text-foreground">{newApplication.display_name}</span><span class="text-success">Verified picker identity</span></div>
                  <p class="mt-1 truncate" title={newApplication.path}>{newApplication.path}</p>
                </div>
              {/if}
            </fieldset>

            <fieldset class="space-y-2">
              <legend class="text-xs font-semibold text-foreground">And any of these agents is active <span class="font-normal text-muted-foreground">(optional)</span></legend>
              <p class="text-meta text-muted-foreground">Select one or more. The app must be open and at least one selected agent must be active. Leave empty for an app-only rule.</p>
              <div class="grid grid-cols-1 gap-2 sm:grid-cols-2" role="group" aria-label="Known Keep Awake agents">
                {#each AWAKE_AGENT_OPTIONS as agent}
                  {@const selected = selectedAgents.includes(agent.id)}
                  <button type="button" aria-pressed={selected} onclick={() => (selectedAgents = toggleAwakeAgent(selectedAgents, agent.id))} class="flex items-center gap-2 rounded-lg border px-3 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {selected ? 'border-ai/40 bg-ai/10 text-foreground' : 'border-border/70 text-muted-foreground hover:bg-secondary/50'}">
                    <span class="flex h-4 w-4 items-center justify-center rounded border {selected ? 'border-ai bg-ai text-white' : 'border-border'}" aria-hidden="true">{#if selected}<Check size={11} />{/if}</span>
                    <span class="min-w-0"><span class="block text-xs font-medium">{agent.label}</span><span class="block text-caption text-muted-foreground">{agent.hint}</span></span>
                  </button>
                {/each}
              </div>
              {#if selectedAgents.length > 0}
                <div class="flex flex-wrap items-center gap-1.5 pt-1" aria-label="Selected agents">
                  <span class="text-caption text-muted-foreground">Any of:</span>
                  {#each selectedAgents as agentId}
                    <span class="inline-flex items-center gap-1 rounded-full border border-ai/25 bg-ai/10 pl-2 pr-1 py-0.5 text-caption text-ai">
                      {awakeAgentLabel(agentId)}
                      <button type="button" onclick={() => removeAgent(agentId)} class="rounded-full p-0.5 hover:bg-ai/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" title={`Remove ${awakeAgentLabel(agentId)}`} aria-label={`Remove ${awakeAgentLabel(agentId)}`}><X size={11} aria-hidden="true" /></button>
                    </span>
                  {/each}
                </div>
              {/if}
            </fieldset>

            <fieldset class="space-y-2">
              <legend class="text-xs font-semibold text-foreground">Power</legend>
              <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
                <label class="flex cursor-pointer gap-2 rounded-lg border p-3 transition-colors {newPowerCondition === 'ac_power_only' ? 'border-success/40 bg-success/5' : 'border-border/70 hover:bg-secondary/40'}">
                  <input type="radio" name="rule-power" value="ac_power_only" bind:group={newPowerCondition} class="mt-0.5 accent-success" />
                  <span><span class="block text-xs font-medium">Plugged in only</span><span class="mt-0.5 block text-caption text-muted-foreground">Recommended to avoid battery drain. Unknown power is ineligible.</span></span>
                </label>
                <label class="flex cursor-pointer gap-2 rounded-lg border p-3 transition-colors {newPowerCondition === 'always' ? 'border-warning/40 bg-warning/5' : 'border-border/70 hover:bg-secondary/40'}">
                  <input type="radio" name="rule-power" value="always" bind:group={newPowerCondition} class="mt-0.5 accent-warning" />
                  <span><span class="block text-xs font-medium">AC or battery</span><span class="mt-0.5 block text-caption text-muted-foreground">Use only when keeping work alive on battery is intentional.</span></span>
                </label>
              </div>
            </fieldset>

            <fieldset class="space-y-2">
              <legend class="text-xs font-semibold text-foreground">Sleep behavior</legend>
              <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
                <label class="flex cursor-pointer gap-2 rounded-lg border p-3 transition-colors {newBehavior === 'prevent_system_sleep' ? 'border-primary/40 bg-primary/5' : 'border-border/70 hover:bg-secondary/40'}">
                  <input type="radio" name="rule-behavior" value="prevent_system_sleep" bind:group={newBehavior} class="mt-0.5 accent-primary" />
                  <span><span class="block text-xs font-medium">Computer awake</span><span class="mt-0.5 block text-caption text-muted-foreground">Work continues; the display may sleep.</span></span>
                </label>
                <label class="flex cursor-pointer gap-2 rounded-lg border p-3 transition-colors {newBehavior === 'keep_display_awake' ? 'border-primary/40 bg-primary/5' : 'border-border/70 hover:bg-secondary/40'}">
                  <input type="radio" name="rule-behavior" value="keep_display_awake" bind:group={newBehavior} class="mt-0.5 accent-primary" />
                  <span><span class="block text-xs font-medium">Computer + display awake</span><span class="mt-0.5 block text-caption text-muted-foreground">Use when the visible display must remain on.</span></span>
                </label>
              </div>
            </fieldset>

            {#if newApplication}
              <div class="rounded-lg border border-border/70 bg-secondary/25 p-3" aria-live="polite">
                <p class="text-caption font-semibold uppercase tracking-wider text-muted-foreground">Before saving</p>
                <p class="mt-1 text-xs leading-relaxed text-foreground">{awakeRuleSummary({ id: 'preview', app_name: newApplication.display_name, executable_pattern: newApplication.executable_name, requires_process_pattern: null, application: newApplication, agent_ids: selectedAgents, behavior: newBehavior, power_condition: newPowerCondition, enabled: false })}</p>
                <p class="mt-1 text-caption text-warning">New rules are saved off. Turn the rule on after reviewing it.</p>
              </div>
            {/if}
          </div>
        {:else}
          <div class="space-y-4 pt-5">
            <div class="rounded-lg border border-warning/25 bg-warning/5 p-3 text-meta text-muted-foreground">Legacy matching may inspect process name, executable path, and command line. Keep this path for older/custom workflows; typed app rules are safer for known agents.</div>
            <div class="space-y-1.5"><label for="legacy-app-name" class="text-xs font-medium text-muted-foreground">Rule name</label><input id="legacy-app-name" type="text" bind:value={newAppName} placeholder="e.g. Blender render" class="h-9 w-full rounded-lg border border-border bg-background px-3 text-xs text-foreground focus:outline-none focus:ring-1 focus:ring-ring" /></div>
            <div class="space-y-1.5"><label for="legacy-primary" class="text-xs font-medium text-muted-foreground">Executable fragments</label><input id="legacy-primary" type="text" bind:value={newExecutable} placeholder="custom process fragment" class="h-9 w-full rounded-lg border border-border bg-background px-3 font-mono text-xs text-foreground focus:outline-none focus:ring-1 focus:ring-ring" /></div>
            <div class="space-y-1.5"><label for="legacy-required" class="text-xs font-medium text-muted-foreground">Also require process (optional)</label><input id="legacy-required" type="text" bind:value={newRequiresPattern} placeholder="secondary fragment or pipe-separated OR values" class="h-9 w-full rounded-lg border border-border bg-background px-3 font-mono text-xs text-foreground focus:outline-none focus:ring-1 focus:ring-ring" /></div>
            <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <label class="flex cursor-pointer gap-2 rounded-lg border border-border/70 p-3"><input type="radio" name="legacy-power" value="ac_power_only" bind:group={newPowerCondition} class="mt-0.5 accent-success" /><span class="text-xs">Plugged in only</span></label>
              <label class="flex cursor-pointer gap-2 rounded-lg border border-border/70 p-3"><input type="radio" name="legacy-power" value="always" bind:group={newPowerCondition} class="mt-0.5 accent-warning" /><span class="text-xs">AC or battery</span></label>
            </div>
            <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <label class="flex cursor-pointer gap-2 rounded-lg border border-border/70 p-3"><input type="radio" name="legacy-behavior" value="prevent_system_sleep" bind:group={newBehavior} class="mt-0.5 accent-primary" /><span class="text-xs">Computer awake; display may sleep</span></label>
              <label class="flex cursor-pointer gap-2 rounded-lg border border-border/70 p-3"><input type="radio" name="legacy-behavior" value="keep_display_awake" bind:group={newBehavior} class="mt-0.5 accent-primary" /><span class="text-xs">Computer + display awake</span></label>
            </div>
          </div>
        {/if}

        {#if pickerError}
          <p class="mt-4 rounded-lg border border-destructive/20 bg-destructive/5 px-3 py-2 text-meta text-destructive" role="alert">{pickerError}</p>
        {/if}

        <div class="flex justify-end gap-2 border-t border-border/60 pt-4 mt-5">
          <Button variant="ghost" size="sm" onclick={closeRuleEditor} disabled={isSavingRule}>Cancel</Button>
          <Button variant="primary" size="sm" onclick={() => void saveRule()} disabled={!isEditorValid() || isSavingRule}>{isSavingRule ? 'Saving…' : editingRuleId ? 'Save changes' : 'Save rule'}</Button>
        </div>
      </div>
    </div>
  {/if}
</div>
