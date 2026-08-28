<script lang="ts">
  import { onMount } from 'svelte';
  import type { AiControlPreferences, DashboardTab, ProviderObservation } from '../../lib/models/types';
  import { aiControlStore } from '../../lib/stores/aiControl.svelte';
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import { formatBytes } from '../../lib/utils/format';
  import Button from '../../lib/components/Button.svelte';
  import Card from '../../lib/components/Card.svelte';
  import ProgressBar from '../../lib/components/ProgressBar.svelte';
  import Switch from '../../lib/components/Switch.svelte';
  import { Activity, Battery, Bot, GitBranch, RefreshCw, ShieldCheck, Sparkles, X } from 'lucide-svelte';

  interface Props { onNavigateTab?: (tab: DashboardTab) => void }
  let { onNavigateTab }: Props = $props();
  let preferences = $derived(settingsStore.settings.ai_control);
  let manualProvider = $state('openai-api');
  let manualSpent = $state('');
  let manualLimit = $state('');
  let budgetProvider = $state('openai-api');
  let budgetLimit = $state('20');
  let selectedSection = $state<'overview' | 'usage' | 'autopilot' | 'safety'>('overview');

  onMount(() => { void aiControlStore.refresh(); });

  const dollars = (micros?: number | null) => micros == null ? '—' : `$${(micros / 1_000_000).toFixed(2)}`;
  const compact = new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 });
  const titleCase = (value: string) => value.replaceAll('_', ' ').replace(/\b\w/g, (letter) => letter.toUpperCase());

  function metricValue(provider: ProviderObservation) {
    const metric = provider.metrics[0];
    if (!metric) return 'No value';
    if (metric.used_basis_points != null) return `${Math.round(metric.used_basis_points / 100)}% used`;
    if (metric.cost) return dollars(metric.cost.micros);
    if (metric.tokens != null) return `${compact.format(metric.tokens)} tokens`;
    return 'No value';
  }

  async function updateAutopilot(key: keyof NonNullable<AiControlPreferences['autopilot']>, value: boolean) {
    await aiControlStore.savePreferences({
      ...preferences,
      autopilot: { ...preferences.autopilot, [key]: value },
    });
    await settingsStore.load(true);
  }

  async function addBudget() {
    const limit = Number(budgetLimit);
    if (!Number.isFinite(limit) || limit <= 0) return;
    await aiControlStore.savePreferences({
      ...preferences,
      budgets: [
        ...(preferences.budgets ?? []).filter((budget) => budget.provider_id !== budgetProvider),
        { id: `budget-${budgetProvider}`, provider_id: budgetProvider, period: 'monthly', limit: { micros: Math.round(limit * 1_000_000), currency: 'USD' }, threshold_percents: [50, 80, 100], enabled: true },
      ],
    });
    await settingsStore.load(true);
  }

  async function saveManualUsage() {
    const spent = Number(manualSpent);
    const limit = manualLimit === '' ? null : Number(manualLimit);
    if (!Number.isFinite(spent) || spent < 0 || (limit != null && (!Number.isFinite(limit) || limit <= 0))) return;
    await aiControlStore.savePreferences({
      ...preferences,
      manual_usage: [
        ...(preferences.manual_usage ?? []).filter((entry) => entry.provider_id !== manualProvider),
        { provider_id: manualProvider, spent: { micros: Math.round(spent * 1_000_000), currency: 'USD' }, limit: limit == null ? null : { micros: Math.round(limit * 1_000_000), currency: 'USD' }, resets_at: null, entered_at: Math.floor(Date.now() / 1000) },
      ],
    });
    await settingsStore.load(true);
  }

  async function openRecommendation(id: string) {
    await aiControlStore.createPreview(id);
  }

  async function confirmRecommendation() {
    const destination = await aiControlStore.consumePreview();
    if (destination && onNavigateTab) onNavigateTab(destination as DashboardTab);
  }
</script>

<div class="space-y-6">
  <div class="flex items-center justify-between border-b border-border/60 pb-3">
    <div class="flex items-center gap-3">
      <div class="flex h-9 w-9 items-center justify-center rounded-lg bg-violet-500/10 text-violet-400"><Sparkles size={19} /></div>
      <div><h2 class="text-base font-semibold tracking-tight">AI Control Center</h2><p class="mt-0.5 text-xs text-muted-foreground">Provenance-aware usage, verified sessions, and advisory safety controls</p></div>
    </div>
    <Button variant="outline" size="sm" class="gap-1.5" disabled={aiControlStore.isLoading} onclick={() => aiControlStore.refresh(true)}><RefreshCw size={13} class={aiControlStore.isLoading ? 'animate-gentle-spin' : ''} />Refresh</Button>
  </div>

  <div class="flex gap-1 rounded-lg bg-secondary/50 p-1" aria-label="Control Center sections">
    {#each [['overview', 'Overview'], ['usage', 'Usage & Budgets'], ['autopilot', 'Resource Autopilot'], ['safety', 'Safety Posture']] as section}
      <button type="button" class="flex-1 rounded-md px-3 py-1.5 text-xs font-medium transition-colors {selectedSection === section[0] ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}" onclick={() => selectedSection = section[0] as typeof selectedSection}>{section[1]}</button>
    {/each}
  </div>

  {#if aiControlStore.error}<div class="rounded-xl border border-destructive/20 bg-destructive/5 p-4 text-xs text-destructive">{aiControlStore.error}</div>{/if}
  {#if aiControlStore.isLoading && !aiControlStore.snapshot}<div class="py-20 text-center text-xs text-muted-foreground"><RefreshCw size={22} class="mx-auto mb-3 animate-gentle-spin" />Building a verified snapshot…</div>
  {:else if aiControlStore.snapshot}
    {@const snapshot = aiControlStore.snapshot}
    {#if snapshot.partial_errors.length}<div class="rounded-xl border border-warning/20 bg-warning/5 p-3 text-xs text-warning">Partial snapshot: {snapshot.partial_errors.join(' · ')}</div>{/if}

    {#if selectedSection === 'overview'}
      <div class="grid grid-cols-4 gap-3">
        <Card><p class="text-caption uppercase text-muted-foreground">Verified sessions</p><p class="mt-1 font-mono text-2xl font-semibold">{snapshot.resources.length}</p></Card>
        <Card><p class="text-caption uppercase text-muted-foreground">Provider sources</p><p class="mt-1 font-mono text-2xl font-semibold">{snapshot.providers.filter((p) => p.quality !== 'unavailable').length}</p></Card>
        <Card><p class="text-caption uppercase text-muted-foreground">Zenith alerts</p><p class="mt-1 font-mono text-2xl font-semibold">{snapshot.quick_summary.budget_alerts}</p></Card>
        <Card><p class="text-caption uppercase text-muted-foreground">Safety findings</p><p class="mt-1 font-mono text-2xl font-semibold">{snapshot.quick_summary.safety_findings}</p></Card>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <Card class="space-y-3"><h3 class="text-sm font-semibold">Recommendations</h3>
          {#if snapshot.recommendations.length === 0}<p class="text-xs text-muted-foreground">No advisory actions right now.</p>{/if}
          {#each snapshot.recommendations as recommendation}<div class="flex items-start justify-between gap-3 rounded-lg bg-secondary/40 p-3"><div><p class="text-xs font-medium">{recommendation.title}</p><p class="mt-1 text-caption leading-relaxed text-muted-foreground">{recommendation.message}</p></div>{#if recommendation.action_label}<Button variant="outline" size="sm" onclick={() => openRecommendation(recommendation.id)}>{recommendation.action_label}</Button>{/if}</div>{/each}
        </Card>
        <Card class="space-y-3"><h3 class="text-sm font-semibold">Recent local audit</h3>{#if snapshot.audit.length === 0}<p class="text-xs text-muted-foreground">No Control Center actions recorded.</p>{/if}{#each snapshot.audit.slice(0, 6) as entry}<div class="flex justify-between gap-3 text-caption"><span class="truncate">{entry.message}</span><span class="shrink-0 font-mono text-muted-foreground">{entry.outcome}</span></div>{/each}<p class="text-micro text-muted-foreground">Bounded local log · no telemetry · opaque project references</p></Card>
      </div>
    {:else if selectedSection === 'usage'}
      <div class="grid grid-cols-2 gap-3">
        {#each snapshot.providers as provider}
          <Card class="space-y-3"><div class="flex items-start justify-between gap-3"><div><h3 class="text-sm font-semibold">{provider.display_name}</h3><p class="text-caption text-muted-foreground">{titleCase(provider.scope)} · {titleCase(provider.source_kind)}</p></div><span class="rounded-full border border-border px-2 py-0.5 text-caption {provider.quality === 'fresh' ? 'text-success' : provider.quality === 'partial' || provider.quality === 'stale' ? 'text-warning' : 'text-muted-foreground'}">{titleCase(provider.quality)}</span></div><div class="font-mono text-lg font-semibold">{metricValue(provider)}</div><p class="text-caption leading-relaxed text-muted-foreground">{provider.status_message}</p><p class="text-micro text-muted-foreground">{provider.period.label}{provider.partial_error ? ` · ${provider.partial_error}` : ''}</p></Card>
        {/each}
      </div>
      <div class="grid grid-cols-2 gap-3">
        <Card class="space-y-3"><h3 class="text-sm font-semibold">Zenith local budget alert</h3><p class="text-caption text-muted-foreground">A local alert only. It does not change provider billing or limits.</p><div class="grid grid-cols-2 gap-2"><select bind:value={budgetProvider} class="rounded-md border border-border bg-background px-2 py-1.5 text-xs"><option value="openai-api">OpenAI API</option><option value="openrouter">OpenRouter</option><option value="anthropic-api">Anthropic API</option><option value="xai-api">xAI API</option></select><input bind:value={budgetLimit} aria-label="Monthly budget in USD" inputmode="decimal" class="rounded-md border border-border bg-background px-2 py-1.5 text-xs" placeholder="Monthly USD" /></div><Button size="sm" onclick={addBudget}>Save local alert</Button>{#each snapshot.budget_statuses as status}<div class="space-y-1"><div class="flex justify-between text-caption"><span>{status.source_label}{status.mixed_sources ? ' · mixed sources' : ''}</span><span>{dollars(status.spent.micros)} / {dollars(status.limit.micros)}</span></div><ProgressBar value={status.used_basis_points / 100} height="h-1.5" /></div>{/each}</Card>
        <Card class="space-y-3"><h3 class="text-sm font-semibold">Manual provider entry</h3><p class="text-caption text-muted-foreground">Stored locally and always labelled Manual.</p><select bind:value={manualProvider} class="w-full rounded-md border border-border bg-background px-2 py-1.5 text-xs"><option value="claude-individual">Claude individual</option><option value="cursor-individual">Cursor individual</option><option value="grok-individual">Grok individual</option><option value="gemini-enterprise">Gemini enterprise</option><option value="openai-api">OpenAI API</option></select><div class="grid grid-cols-2 gap-2"><input bind:value={manualSpent} aria-label="Manual spend in USD" inputmode="decimal" class="rounded-md border border-border bg-background px-2 py-1.5 text-xs" placeholder="Spent USD" /><input bind:value={manualLimit} aria-label="Manual limit in USD" inputmode="decimal" class="rounded-md border border-border bg-background px-2 py-1.5 text-xs" placeholder="Limit (optional)" /></div><Button size="sm" onclick={saveManualUsage}>Save manual value</Button></Card>
      </div>
    {:else if selectedSection === 'autopilot'}
      <Card class="space-y-4"><div><h3 class="text-sm font-semibold">Explicit automation</h3><p class="mt-1 text-caption text-muted-foreground">Off by default. Recommendations never terminate processes, close ports, or clean files.</p></div>
        {#each [['keep_awake_for_verified_sessions', 'Keep Awake for verified sessions', 'Uses canonical session identity and releases when the session ends.'], ['keep_awake_ac_only', 'AC power only', 'Unknown power state fails closed.'], ['notify_on_battery', 'Battery recommendation notifications', 'Advisory only.'], ['notify_on_memory_pressure', 'Memory pressure notifications', 'Advisory only.'], ['notify_on_session_completion', 'Session completion notifications', 'Advisory only.']] as option}
          <div class="flex items-center justify-between gap-4 border-t border-border/50 pt-3"><div><p class="text-xs font-medium">{option[1]}</p><p class="text-caption text-muted-foreground">{option[2]}</p></div><Switch checked={Boolean(preferences.autopilot?.[option[0] as keyof NonNullable<AiControlPreferences['autopilot']>])} ariaLabel={option[1]} onchange={(checked) => updateAutopilot(option[0] as keyof NonNullable<AiControlPreferences['autopilot']>, checked)} /></div>
        {/each}
      </Card>
      <div class="grid grid-cols-2 gap-3">{#each snapshot.resources as resource}<Card class="space-y-2"><div class="flex justify-between"><h3 class="text-sm font-semibold">{resource.tool_name}</h3><span class="text-caption {resource.mutable_actions_allowed ? 'text-success' : 'text-warning'}">{resource.mutable_actions_allowed ? 'Verified' : 'Unassigned'}</span></div><div class="grid grid-cols-3 gap-2 text-caption"><span>{formatBytes(resource.memory_bytes)}</span><span>{resource.cpu_percent == null ? 'CPU —' : `CPU ${resource.cpu_percent.toFixed(1)}%`}</span><span>{resource.open_dev_ports} ports</span></div><p class="text-caption text-muted-foreground">{resource.reason}</p></Card>{/each}</div>
    {:else}
      <div class="flex items-center justify-between rounded-xl border border-border bg-card p-4"><div class="flex gap-3"><ShieldCheck size={18} class="text-success" /><div><p class="text-sm font-semibold">Bounded, redacted inspection</p><p class="text-caption text-muted-foreground">Registered project roots only; symlinks, raw secrets, arguments, headers, environment values, and email addresses are excluded.</p></div></div><Button variant="outline" size="sm" disabled={aiControlStore.isScanning} onclick={() => aiControlStore.scanSafety()}>{aiControlStore.isScanning ? 'Inspecting…' : 'Run inspection'}</Button></div>
      <div class="grid grid-cols-2 gap-3">{#if snapshot.safety.findings.filter((finding) => !finding.dismissed).length === 0}<Card class="col-span-2 text-center"><p class="text-xs text-muted-foreground">{snapshot.safety.status_message}</p></Card>{/if}{#each snapshot.safety.findings.filter((finding) => !finding.dismissed) as finding}<Card class="space-y-2"><div class="flex justify-between"><span class="text-xs font-semibold">{titleCase(finding.kind)}</span><span class="text-caption uppercase text-warning">{finding.severity}</span></div><p class="text-caption text-muted-foreground">{finding.remediation}</p><div class="flex items-center justify-between"><span class="font-mono text-micro text-muted-foreground">{finding.relative_path ?? 'project scope'}{finding.line_start ? `:${finding.line_start}` : ''}</span><Button variant="ghost" size="sm" onclick={() => aiControlStore.dismissFinding(finding.id)}>Dismiss</Button></div></Card>{/each}</div>
      <Card class="space-y-3"><div><h3 class="text-sm font-semibold">Git changes since Zenith baseline</h3><p class="text-caption text-muted-foreground">Metadata only. Pre-existing changes are excluded; diffs are fetched explicitly and never persisted.</p></div>{#each snapshot.git_summaries as git}<div class="flex items-center justify-between gap-3 rounded-lg bg-secondary/40 p-3"><div><p class="text-xs font-medium"><GitBranch size={12} class="mr-1 inline" />{git.status_message}</p><p class="mt-1 font-mono text-micro text-muted-foreground">+{git.added} ~{git.modified} -{git.deleted} R{git.renamed} ?{git.untracked}</p></div>{#if git.available}<Button variant="outline" size="sm" onclick={() => aiControlStore.loadGitDiff(git.project_id)}>View ephemeral diff</Button>{/if}</div>{/each}</Card>
    {/if}
  {/if}
</div>

{#if aiControlStore.preview}<div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-8"><Card class="w-full max-w-md space-y-4"><div class="flex items-start justify-between"><div><h3 class="text-sm font-semibold">{aiControlStore.preview.title}</h3><p class="mt-2 text-xs leading-relaxed text-muted-foreground">{aiControlStore.preview.explanation}</p></div><button type="button" aria-label="Close preview" onclick={() => aiControlStore.preview = null}><X size={16} /></button></div><p class="text-caption text-warning"><Battery size={13} class="mr-1 inline" />One-shot preview expires automatically. No action has run.</p><Button onclick={confirmRecommendation}>{aiControlStore.preview.action_label || ('Open ' + titleCase(aiControlStore.preview.destination))}</Button></Card></div>{/if}
{#if aiControlStore.gitDiff != null}<div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-8"><Card class="flex max-h-[80vh] w-full max-w-3xl flex-col gap-3"><div class="flex items-center justify-between"><h3 class="text-sm font-semibold">Ephemeral Git diff</h3><button type="button" aria-label="Close Git diff" onclick={() => aiControlStore.clearGitDiff()}><X size={16} /></button></div><pre class="overflow-auto whitespace-pre-wrap rounded-lg bg-secondary/50 p-3 text-micro select-text">{aiControlStore.gitDiff || 'No post-baseline diff.'}</pre></Card></div>{/if}
