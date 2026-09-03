<script lang="ts">
  import { onMount } from 'svelte';
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import type {
    AgentNotificationPreferences,
    AiProviderId,
    DashboardTab,
    DiagnosticsSnapshot,
    QuickPanelSection,
  } from '../../lib/models/types';
  import { tauriGetDiagnostics, tauriOpenLogsFolder } from '../../lib/utils/tauri';
  import Card from '../../lib/components/Card.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import Button from '../../lib/components/Button.svelte';
  import Switch from '../../lib/components/Switch.svelte';
  import Checkbox from '../../lib/components/Checkbox.svelte';
  import ReorderControls from '../../lib/components/ReorderControls.svelte';
  import { APP_VERSION, formatVersion } from '../../lib/utils/version';
  import {
    Settings,
    Sparkles,
    Moon,
    Sun,
    Monitor,
    PanelTop,
    LayoutList,
    GripVertical,
    FolderOpen,
    FileText,
    AlertTriangle,
    Bell,
    Users,
  } from 'lucide-svelte';

  const tabOptions: { id: DashboardTab; label: string; description: string }[] = [
    { id: 'storage', label: 'Storage & Disks', description: 'Primary storage, volumes, and developer/AI caches.' },
    { id: 'docker', label: 'Containers', description: 'Docker images, build cache, stopped containers, and volumes.' },
    { id: 'models', label: 'Local Models', description: 'Ollama, HuggingFace, LM Studio, and Apple MLX models.' },
    { id: 'memory', label: 'Memory', description: 'Memory pressure, top processes, and resource guard.' },
    { id: 'development_servers', label: 'Development Servers', description: 'Inspect and safely release verified local TCP listeners.' },
    { id: 'projects', label: 'AI Activity', description: 'Active AI agent sessions, dev listeners, and account token limits.' },
    { id: 'awake', label: 'Keep Awake', description: 'Prevent system and display sleep rules.' },
  ];

  const sectionOptions: { id: QuickPanelSection; label: string; description: string }[] = [
    { id: 'cleanup', label: 'Quick Clean', description: 'Safe reclaimable storage and clean action.' },
    { id: 'storage', label: 'Storage', description: 'Primary disk capacity and usage.' },
    { id: 'memory', label: 'Memory', description: 'Memory pressure and current usage.' },
    { id: 'categories', label: 'Storage Categories', description: 'AI, developer, container, model, and system totals.' },
    { id: 'agent_activity', label: 'AI & Agents', description: 'Active AI agent sessions and account token limits.' },
  ];
  const quickPanelProviderOptions: { id: AiProviderId; label: string }[] = [
    { id: 'codex', label: 'Codex' },
    { id: 'claude', label: 'Claude Code' },
    { id: 'opencode', label: 'OpenCode' },
    { id: 'openrouter', label: 'OpenRouter' },
    { id: 'antigravity', label: 'Antigravity' },
  ];
  const accountProviderOptions: { id: AiProviderId; label: string; description: string }[] = [
    { id: 'codex', label: 'Codex', description: 'Live ChatGPT account limits through the official app server.' },
    { id: 'claude', label: 'Claude Code', description: 'Local availability with quota checked in Claude /usage.' },
    { id: 'opencode', label: 'OpenCode', description: 'Local sessions and cost from connected providers.' },
    { id: 'openrouter', label: 'OpenRouter', description: 'Live key usage through Zenith OAuth.' },
    { id: 'antigravity', label: 'Antigravity', description: 'Live Gemini and Claude/GPT limits from agy.' },
    { id: 'cursor', label: 'Cursor', description: 'Local availability; quota stays in Cursor settings.' },
    { id: 'grok', label: 'Grok Build', description: 'Local availability; quota stays in the provider client.' },
  ];

  let settings = $derived(settingsStore.settings);

  let draggedTab = $state<DashboardTab | null>(null);
  let dragOverTab = $state<DashboardTab | null>(null);

  let draggedSection = $state<QuickPanelSection | null>(null);
  let dragOverSection = $state<QuickPanelSection | null>(null);

  let draggedProvider = $state<AiProviderId | null>(null);
  let dragOverProvider = $state<AiProviderId | null>(null);
  let draggedAccountProvider = $state<AiProviderId | null>(null);
  let dragOverAccountProvider = $state<AiProviderId | null>(null);

  function handleToggle(key: keyof typeof settings) {
    if (typeof settings[key] === 'boolean') {
      settingsStore.save({ [key]: !settings[key] });
    }
  }

  function handleTheme(theme: string) {
    settingsStore.save({ theme });
  }

  function handleNotificationToggle(key: keyof AgentNotificationPreferences) {
    const current = settings.agent_notifications ?? {
      enabled: false,
      notify_on_turn_completed: true,
      notify_on_approval_or_input: true,
      notify_on_possibly_inactive: true,
      hide_project_basename: false,
      inactivity_threshold_minutes: 15,
    };
    const updated = {
      ...current,
      [key]: !current[key],
    };
    settingsStore.save({ agent_notifications: updated });
  }

  function handleThresholdChange(minutes: number) {
    const current = settings.agent_notifications ?? {
      enabled: false,
      notify_on_turn_completed: true,
      notify_on_approval_or_input: true,
      notify_on_possibly_inactive: true,
      hide_project_basename: false,
      inactivity_threshold_minutes: 15,
    };
    const updated = {
      ...current,
      inactivity_threshold_minutes: Math.max(5, Math.min(120, minutes)),
    };
    settingsStore.save({ agent_notifications: updated });
  }

  let diagnosticsData = $state<DiagnosticsSnapshot | null>(null);
  let copiedDiagnostics = $state(false);

  onMount(() => {
    void loadDiagnostics();
  });

  async function loadDiagnostics() {
    try {
      diagnosticsData = await tauriGetDiagnostics();
    } catch {
      // preview mode fallback
    }
  }

  async function handleOpenLogs() {
    try {
      await tauriOpenLogsFolder();
    } catch (e: any) {
      settingsStore.error = e?.toString() || 'Failed to open logs folder';
    }
  }

  async function handleExportDiagnostics() {
    try {
      diagnosticsData = await tauriGetDiagnostics();
      if (typeof navigator !== 'undefined' && navigator.clipboard) {
        await navigator.clipboard.writeText(JSON.stringify(diagnosticsData, null, 2));
        copiedDiagnostics = true;
        setTimeout(() => {
          copiedDiagnostics = false;
        }, 2500);
      }
    } catch (e: any) {
      settingsStore.error = e?.toString() || 'Failed to export diagnostics';
    }
  }

  function orderedDashboardTabs() {
    const selected = (settings.dashboard_tabs ?? [])
      .map((id) => tabOptions.find((option) => option.id === id))
      .filter((option): option is (typeof tabOptions)[number] => Boolean(option));
    return [...selected, ...tabOptions.filter((option) => !(settings.dashboard_tabs ?? []).includes(option.id))];
  }

  function orderedSections() {
    const selected = settings.quick_panel_sections
      .map((id) => sectionOptions.find((option) => option.id === id))
      .filter((option): option is (typeof sectionOptions)[number] => Boolean(option));
    return [...selected, ...sectionOptions.filter((option) => !settings.quick_panel_sections.includes(option.id))];
  }

  function orderedQuickPanelProviders() {
    const selected = settings.quick_panel_ai_providers
      .map((id) => quickPanelProviderOptions.find((option) => option.id === id))
      .filter((option): option is (typeof quickPanelProviderOptions)[number] => Boolean(option));
    return [...selected, ...quickPanelProviderOptions.filter((option) => !settings.quick_panel_ai_providers.includes(option.id))];
  }

  function orderedAccountProviders() {
    const selected = settings.ai_accounts_quota_providers
      .map((id) => accountProviderOptions.find((option) => option.id === id))
      .filter((option): option is (typeof accountProviderOptions)[number] => Boolean(option));
    return [...selected, ...accountProviderOptions.filter((option) => !settings.ai_accounts_quota_providers.includes(option.id))];
  }
</script>

<div class="space-y-6 max-w-2xl">
  <!-- Header -->
  <div class="pb-3 border-b border-border/60">
    <div class="flex items-center gap-3">
      <div class="h-9 w-9 rounded-lg bg-secondary text-foreground flex items-center justify-center">
        <Settings size={20} />
      </div>
      <div>
        <h2 class="text-base font-semibold text-foreground tracking-tight">Settings</h2>
        <p class="text-xs text-muted-foreground mt-0.5">
          Configure cleaning defaults, appearance, and system integration.
        </p>
      </div>
    </div>
  </div>

  {#if settingsStore.error}
    <div class="rounded-xl border border-destructive/20 bg-destructive/5 px-4 py-3 text-xs text-destructive">
      {settingsStore.error}
    </div>
  {/if}

  <!-- General Preferences -->
  <div class="space-y-3">
    <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
      General
    </h3>
    <Card class="p-4 space-y-4 bg-card/70">
      <div class="flex items-center justify-between text-xs">
        <div>
          <div class="flex items-center gap-2 font-medium text-foreground">Launch Zenith at login <Badge variant="outline">Planned</Badge></div>
          <div class="text-meta text-muted-foreground">Autostart is not enabled in this build.</div>
        </div>
        <Switch
          checked={settings.launch_at_login}
          disabled={true}
          ariaLabel="Launch Zenith at login"
        />
      </div>
    </Card>
  </div>

  <!-- Dashboard Navigation Customization -->
  <div class="space-y-3">
    <div>
      <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
        Dashboard Navigation Menu
      </h3>
      <p class="text-meta text-muted-foreground mt-1">
        Customize the tabs displayed in the left sidebar. Drag or use the arrow buttons to reorder.
      </p>
    </div>
    <Card class="p-4 bg-card/70 space-y-3">
      <div class="flex items-center gap-2 text-xs font-medium text-foreground pb-1">
        <LayoutList size={14} /> Sidebar Menu Order
      </div>
      {#each orderedDashboardTabs() as tabOption (tabOption.id)}
        {@const enabled = (settings.dashboard_tabs ?? []).includes(tabOption.id)}
        {@const enabledIndex = (settings.dashboard_tabs ?? []).indexOf(tabOption.id)}
        <div
          role="listitem"
          draggable={enabled}
          ondragstart={() => {
            if (enabled) draggedTab = tabOption.id;
          }}
          ondragover={(e) => {
            if (enabled && draggedTab) {
              e.preventDefault();
              dragOverTab = tabOption.id;
            }
          }}
          ondragleave={() => {
            if (dragOverTab === tabOption.id) dragOverTab = null;
          }}
          ondrop={(e) => {
            e.preventDefault();
            if (draggedTab && draggedTab !== tabOption.id) {
              settingsStore.reorderDashboardTabs(draggedTab, tabOption.id);
            }
            draggedTab = null;
            dragOverTab = null;
          }}
          ondragend={() => {
            draggedTab = null;
            dragOverTab = null;
          }}
          class="flex items-center gap-3 rounded-lg border px-3 py-2.5 transition-[background-color,border-color,opacity,transform] {enabled ? 'cursor-grab active:cursor-grabbing bg-card' : 'opacity-60 bg-muted/20'} {dragOverTab === tabOption.id ? 'border-primary bg-primary/5 scale-[1.01]' : 'border-border/60'} {draggedTab === tabOption.id ? 'opacity-40' : ''}"
        >
          <GripVertical size={14} class="text-muted-foreground/60 shrink-0 select-none {enabled ? 'hover:text-foreground' : 'opacity-20'}" />
          <Checkbox
            checked={enabled}
            disabled={enabled && (settings.dashboard_tabs ?? []).length === 1}
            onchange={() => settingsStore.toggleDashboardTab(tabOption.id)}
            ariaLabel={`Show ${tabOption.label} in sidebar`}
          />
          <div class="min-w-0 flex-1 select-none">
            <div class="text-xs font-medium text-foreground">{tabOption.label}</div>
            <div class="text-caption text-muted-foreground">{tabOption.description}</div>
          </div>
          {#if enabled}
            <ReorderControls
              label={tabOption.label}
              index={enabledIndex}
              count={(settings.dashboard_tabs ?? []).length}
              onMove={(direction) => settingsStore.moveDashboardTab(tabOption.id, direction)}
            />
          {/if}
        </div>
      {/each}
    </Card>
  </div>

  <!-- AI Accounts & Quota Customization -->
  <div class="space-y-3">
    <div>
      <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
        AI Accounts & Quota
      </h3>
      <p class="text-meta text-muted-foreground mt-1">
        Choose which account providers Zenith checks and displays. Disabled providers are not queried.
      </p>
    </div>
    <Card class="p-4 bg-card/70 space-y-3">
      <div class="flex items-center gap-2 text-xs font-medium text-foreground pb-1">
        <Users size={14} /> Provider order
      </div>
      {#each orderedAccountProviders() as provider (provider.id)}
        {@const enabled = settings.ai_accounts_quota_providers.includes(provider.id)}
        {@const enabledIndex = settings.ai_accounts_quota_providers.indexOf(provider.id)}
        <div
          role="listitem"
          draggable={enabled}
          ondragstart={() => {
            if (enabled) draggedAccountProvider = provider.id;
          }}
          ondragover={(e) => {
            if (enabled && draggedAccountProvider) {
              e.preventDefault();
              dragOverAccountProvider = provider.id;
            }
          }}
          ondragleave={() => {
            if (dragOverAccountProvider === provider.id) dragOverAccountProvider = null;
          }}
          ondrop={(e) => {
            e.preventDefault();
            if (draggedAccountProvider && draggedAccountProvider !== provider.id) {
              settingsStore.reorderAccountsQuotaProviders(draggedAccountProvider, provider.id);
            }
            draggedAccountProvider = null;
            dragOverAccountProvider = null;
          }}
          ondragend={() => {
            draggedAccountProvider = null;
            dragOverAccountProvider = null;
          }}
          class="flex items-center gap-3 rounded-lg border px-3 py-2.5 transition-[background-color,border-color,opacity,transform] {enabled ? 'cursor-grab active:cursor-grabbing bg-card' : 'opacity-60 bg-muted/20'} {dragOverAccountProvider === provider.id ? 'border-primary bg-primary/5 scale-[1.01]' : 'border-border/60'} {draggedAccountProvider === provider.id ? 'opacity-40' : ''}"
        >
          <GripVertical size={14} class="text-muted-foreground/60 shrink-0 select-none {enabled ? 'hover:text-foreground' : 'opacity-20'}" />
          <Checkbox
            checked={enabled}
            disabled={enabled && settings.ai_accounts_quota_providers.length === 1}
            onchange={() => settingsStore.toggleAccountsQuotaProvider(provider.id)}
            ariaLabel={`Collect and show ${provider.label} account usage`}
          />
          <div class="min-w-0 flex-1 select-none">
            <div class="text-xs font-medium text-foreground">{provider.label}</div>
            <div class="text-caption text-muted-foreground">{provider.description}</div>
          </div>
          {#if enabled}
            <ReorderControls
              label={provider.label}
              index={enabledIndex}
              count={settings.ai_accounts_quota_providers.length}
              onMove={(direction) => settingsStore.moveAccountsQuotaProvider(provider.id, direction)}
            />
          {/if}
        </div>
      {/each}
    </Card>
  </div>

  <!-- Quick Panel Customization -->
  <div class="space-y-3">
    <div>
      <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
        Menu Bar Quick Panel
      </h3>
      <p class="text-meta text-muted-foreground mt-1">
        Choose what appears below the menu bar icon. Drag or use the arrow buttons to set priority.
      </p>
    </div>
    <Card class="p-4 bg-card/70 space-y-5">
      <div class="space-y-2">
        <div class="flex items-center gap-2 text-xs font-medium text-foreground">
          <PanelTop size={14} /> Sections
        </div>
        {#each orderedSections() as option (option.id)}
          {@const enabled = settings.quick_panel_sections.includes(option.id)}
          {@const enabledIndex = settings.quick_panel_sections.indexOf(option.id)}
          <div
            role="listitem"
            draggable={enabled}
            ondragstart={() => {
              if (enabled) draggedSection = option.id;
            }}
            ondragover={(e) => {
              if (enabled && draggedSection) {
                e.preventDefault();
                dragOverSection = option.id;
              }
            }}
            ondragleave={() => {
              if (dragOverSection === option.id) dragOverSection = null;
            }}
            ondrop={(e) => {
              e.preventDefault();
              if (draggedSection && draggedSection !== option.id) {
                settingsStore.reorderQuickPanelSections(draggedSection, option.id);
              }
              draggedSection = null;
              dragOverSection = null;
            }}
            ondragend={() => {
              draggedSection = null;
              dragOverSection = null;
            }}
            class="flex items-center gap-3 rounded-lg border px-3 py-2.5 transition-[background-color,border-color,opacity,transform] {enabled ? 'cursor-grab active:cursor-grabbing bg-card' : 'opacity-60 bg-muted/20'} {dragOverSection === option.id ? 'border-primary bg-primary/5 scale-[1.01]' : 'border-border/60'} {draggedSection === option.id ? 'opacity-40' : ''}"
          >
            <GripVertical size={14} class="text-muted-foreground/60 shrink-0 select-none {enabled ? 'hover:text-foreground' : 'opacity-20'}" />
            <Checkbox
              checked={enabled}
              disabled={enabled && settings.quick_panel_sections.length === 1}
              onchange={() => settingsStore.toggleQuickPanelSection(option.id)}
              ariaLabel={`Show ${option.label} in quick panel`}
            />
            <div class="min-w-0 flex-1 select-none">
              <div class="text-xs font-medium text-foreground">{option.label}</div>
              <div class="text-caption text-muted-foreground">{option.description}</div>
            </div>
            {#if enabled}
              <ReorderControls
                label={option.label}
                index={enabledIndex}
                count={settings.quick_panel_sections.length}
                onMove={(direction) => settingsStore.moveQuickPanelSection(option.id, direction)}
              />
            {/if}
          </div>
        {/each}
      </div>

      <div class="space-y-2 pt-4 border-t border-border/60">
        <div class="flex items-center gap-2 text-xs font-medium text-foreground">
          <Sparkles size={14} /> AI Provider Priority
        </div>
        <p class="text-caption text-muted-foreground">Only enabled providers are displayed in this order. Providers disabled under Accounts & Quota are not loaded.</p>
        {#each orderedQuickPanelProviders() as provider (provider.id)}
          {@const enabled = settings.quick_panel_ai_providers.includes(provider.id)}
          {@const enabledIndex = settings.quick_panel_ai_providers.indexOf(provider.id)}
          <div
            role="listitem"
            draggable={enabled}
            ondragstart={() => {
              if (enabled) draggedProvider = provider.id;
            }}
            ondragover={(e) => {
              if (enabled && draggedProvider) {
                e.preventDefault();
                dragOverProvider = provider.id;
              }
            }}
            ondragleave={() => {
              if (dragOverProvider === provider.id) dragOverProvider = null;
            }}
            ondrop={(e) => {
              e.preventDefault();
              if (draggedProvider && draggedProvider !== provider.id) {
                settingsStore.reorderQuickPanelProviders(draggedProvider, provider.id);
              }
              draggedProvider = null;
              dragOverProvider = null;
            }}
            ondragend={() => {
              draggedProvider = null;
              dragOverProvider = null;
            }}
            class="flex items-center gap-3 rounded-lg border px-3 py-2 transition-[background-color,border-color,opacity,transform] {enabled ? 'cursor-grab active:cursor-grabbing bg-card' : 'opacity-60 bg-muted/20'} {dragOverProvider === provider.id ? 'border-primary bg-primary/5 scale-[1.01]' : 'border-border/60'} {draggedProvider === provider.id ? 'opacity-40' : ''}"
          >
            <GripVertical size={14} class="text-muted-foreground/60 shrink-0 select-none {enabled ? 'hover:text-foreground' : 'opacity-20'}" />
            <Checkbox
              checked={enabled}
              disabled={!settings.ai_accounts_quota_providers.includes(provider.id)}
              onchange={() => settingsStore.toggleQuickPanelProvider(provider.id)}
              ariaLabel={`Show ${provider.label} usage`}
            />
            <span class="flex-1 text-xs font-medium text-foreground select-none">{provider.label}</span>
            {#if enabled}
              <ReorderControls
                label={provider.label}
                index={enabledIndex}
                count={settings.quick_panel_ai_providers.length}
                onMove={(direction) => settingsStore.moveQuickPanelProvider(provider.id, direction)}
              />
            {/if}
          </div>
        {/each}
      </div>
    </Card>
  </div>

  <!-- Cleanup Scan Scope -->
  <div class="space-y-3">
    <div>
      <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
        Cleanup Scan Scope
      </h3>
      <p class="text-meta text-muted-foreground mt-1">
        Choose how broadly Zenith searches for reclaimable cache and log data.
      </p>
    </div>
    <Card class="p-4 bg-card/70">
      <div class="flex items-start justify-between gap-5 text-xs">
        <div class="min-w-0">
          <div class="flex items-center gap-2 font-medium text-foreground">
            <AlertTriangle size={14} class="text-warning" />
            Intensive cleanup
            <Badge variant="outline">Opt-in</Badge>
          </div>
          <div class="text-meta text-muted-foreground mt-1 leading-relaxed">
            Include stale third-party application caches and logs. Apps may rebuild or re-download cached data.
            Personal files, settings, credentials, Apple system caches, and recent temporary data remain protected.
          </div>
        </div>
        <Switch
          checked={settings.intensive_cleanup}
          onchange={() => handleToggle('intensive_cleanup')}
          ariaLabel="Intensive cleanup"
        />
      </div>
    </Card>
  </div>

  <!-- Cleaning Categories Defaults -->
  <div class="space-y-3">
    <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
      Default Quick Clean Categories
    </h3>
    <Card class="p-4 space-y-4 bg-card/70 divide-y divide-border/60">
      <!-- AI Tools -->
      <div class="flex items-center justify-between text-xs pt-3 first:pt-0">
        <div>
          <div class="font-medium text-foreground">AI Assistant Caches & Logs</div>
          <div class="text-meta text-muted-foreground">Claude Code, Cursor, Gemini CLI, Codex, Aider.</div>
        </div>
        <Switch
          checked={settings.clean_ai_tools}
          onchange={() => handleToggle('clean_ai_tools')}
          ariaLabel="AI Assistant Caches & Logs"
        />
      </div>

      <!-- Developer Tools -->
      <div class="flex items-center justify-between text-xs pt-3">
        <div>
          <div class="font-medium text-foreground">Developer Compilers & Package Managers</div>
          <div class="text-meta text-muted-foreground">Go build, Cargo cache, npm, pnpm, uv, Xcode DerivedData.</div>
        </div>
        <Switch
          checked={settings.clean_developer_tools}
          onchange={() => handleToggle('clean_developer_tools')}
          ariaLabel="Developer Compilers & Package Managers"
        />
      </div>

      <!-- Docker -->
      <div class="flex items-center justify-between text-xs pt-3">
        <div>
          <div class="font-medium text-foreground">Docker Dangling Images & BuildKit Cache</div>
          <div class="text-meta text-muted-foreground">Clean safe Docker cache layers via official Docker CLI.</div>
        </div>
        <Switch
          checked={settings.clean_docker}
          onchange={() => handleToggle('clean_docker')}
          ariaLabel="Docker Dangling Images & BuildKit Cache"
        />
      </div>

      <!-- Local Models -->
      <div class="flex items-center justify-between text-xs pt-3">
        <div>
          <div class="font-medium text-foreground">Local Models (Ollama / HuggingFace)</div>
          <div class="text-meta text-muted-foreground">Always off by default to protect stateful weights.</div>
        </div>
        <Switch
          checked={settings.clean_local_models}
          onchange={() => handleToggle('clean_local_models')}
          ariaLabel="Local Models"
        />
      </div>
    </Card>
  </div>

  <!-- Agent Activity Notifications -->
  <div class="space-y-3">
    <div class="flex items-center justify-between">
      <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
        Agent Activity Notifications
      </h3>
      <Badge variant="outline">Privacy Safe</Badge>
    </div>
    <Card class="p-4 space-y-4 bg-card/70 divide-y divide-border/60">
      <!-- Master toggle -->
      <div class="flex items-center justify-between text-xs pt-3 first:pt-0">
        <div>
          <div class="font-medium text-foreground">Enable Desktop Notifications</div>
          <div class="text-meta text-muted-foreground">Opt-in notifications for agent lifecycle and user attention events.</div>
        </div>
        <Switch
          checked={settings.agent_notifications?.enabled ?? false}
          onchange={() => handleNotificationToggle('enabled')}
          ariaLabel="Enable Desktop Notifications"
        />
      </div>

      {#if settings.agent_notifications?.enabled}
        <!-- Turn completed -->
        <div class="flex items-center justify-between text-xs pt-3">
          <div>
            <div class="font-medium text-foreground">Turn Completed</div>
            <div class="text-meta text-muted-foreground">Notify when an agent finishes its active response turn.</div>
          </div>
          <Switch
            checked={settings.agent_notifications?.notify_on_turn_completed ?? true}
            onchange={() => handleNotificationToggle('notify_on_turn_completed')}
            ariaLabel="Notify on Turn Completed"
          />
        </div>

        <!-- Approval or input needed -->
        <div class="flex items-center justify-between text-xs pt-3">
          <div>
            <div class="font-medium text-foreground">Needs Approval or Input</div>
            <div class="text-meta text-muted-foreground">Notify when an agent is waiting for confirmation, tool permission, or user input.</div>
          </div>
          <Switch
            checked={settings.agent_notifications?.notify_on_approval_or_input ?? true}
            onchange={() => handleNotificationToggle('notify_on_approval_or_input')}
            ariaLabel="Notify on Approval or Input"
          />
        </div>

        <!-- Possibly inactive -->
        <div class="flex items-center justify-between text-xs pt-3">
          <div>
            <div class="font-medium text-foreground">Possibly Inactive</div>
            <div class="text-meta text-muted-foreground">Notify if an agent has been running with no observable activity past the threshold.</div>
          </div>
          <Switch
            checked={settings.agent_notifications?.notify_on_possibly_inactive ?? true}
            onchange={() => handleNotificationToggle('notify_on_possibly_inactive')}
            ariaLabel="Notify on Possibly Inactive"
          />
        </div>

        <!-- Hide project basename -->
        <div class="flex items-center justify-between text-xs pt-3">
          <div>
            <div class="font-medium text-foreground">Hide Project Name in Notifications</div>
            <div class="text-meta text-muted-foreground">Replaces the project folder name with "an active project" in notification body text.</div>
          </div>
          <Switch
            checked={settings.agent_notifications?.hide_project_basename ?? false}
            onchange={() => handleNotificationToggle('hide_project_basename')}
            ariaLabel="Hide Project Name in Notifications"
          />
        </div>

        <!-- Inactivity threshold slider -->
        <div class="text-xs pt-3 space-y-2">
          <div class="flex items-center justify-between">
            <span class="font-medium text-foreground">Inactivity Alert Threshold</span>
            <span class="font-mono text-muted-foreground">{settings.agent_notifications?.inactivity_threshold_minutes ?? 15} minutes</span>
          </div>
          <input
            type="range"
            min="5"
            max="60"
            step="5"
            value={settings.agent_notifications?.inactivity_threshold_minutes ?? 15}
            oninput={(e) => handleThresholdChange(Number((e.target as HTMLInputElement).value))}
            class="w-full h-1.5 bg-secondary rounded-lg appearance-none cursor-pointer accent-primary"
          />
        </div>
      {/if}
    </Card>
  </div>

  <!-- Appearance -->
  <div class="space-y-3">
    <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
      Appearance
    </h3>
    <Card class="p-4 bg-card/70">
      <div class="grid grid-cols-3 gap-3">
        <button
          type="button"
          onclick={() => handleTheme('system')}
          class="flex flex-col items-center justify-center p-3 rounded-lg border text-xs gap-2 transition-[background-color,color,border-color] {settings.theme ===
          'system'
            ? 'border-primary bg-secondary/80 text-foreground font-semibold'
            : 'border-border text-muted-foreground hover:text-foreground'}"
        >
          <Monitor size={18} />
          <span>System</span>
        </button>

        <button
          type="button"
          onclick={() => handleTheme('dark')}
          class="flex flex-col items-center justify-center p-3 rounded-lg border text-xs gap-2 transition-[background-color,color,border-color] {settings.theme ===
          'dark'
            ? 'border-primary bg-secondary/80 text-foreground font-semibold'
            : 'border-border text-muted-foreground hover:text-foreground'}"
        >
          <Moon size={18} />
          <span>Dark</span>
        </button>

        <button
          type="button"
          onclick={() => handleTheme('light')}
          class="flex flex-col items-center justify-center p-3 rounded-lg border text-xs gap-2 transition-[background-color,color,border-color] {settings.theme ===
          'light'
            ? 'border-primary bg-secondary/80 text-foreground font-semibold'
            : 'border-border text-muted-foreground hover:text-foreground'}"
        >
          <Sun size={18} />
          <span>Light</span>
        </button>
      </div>
    </Card>
  </div>

  <!-- Diagnostics & Logs -->
  <div class="space-y-3">
    <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
      Diagnostics & Privacy Logs
    </h3>
    <Card class="p-4 bg-card/70 space-y-4">
      <div class="space-y-1">
        <div class="text-xs font-medium text-foreground">Local System & Error Logs</div>
        <p class="text-meta text-muted-foreground leading-relaxed">
          Zenith keeps zero telemetry and never transmits analytics or secrets. Error and subprocess failure logs are stored locally on your machine at <code class="font-mono text-caption bg-secondary/80 px-1 py-0.5 rounded">~/Library/Logs/Zenith</code>.
        </p>
      </div>

      {#if diagnosticsData?.settings_corrupt_recovered}
        <div class="flex items-center gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
          <AlertTriangle size={14} class="shrink-0" />
          <span>A damaged configuration file was detected, safely backed up, and reset to defaults.</span>
        </div>
      {/if}

      <div class="flex flex-wrap gap-2 pt-1">
        <Button variant="secondary" size="sm" onclick={handleOpenLogs}>
          <FolderOpen size={14} />
          <span>Open Logs Folder</span>
        </Button>
        <Button variant="secondary" size="sm" onclick={handleExportDiagnostics}>
          <FileText size={14} />
          <span>{copiedDiagnostics ? 'Copied Diagnostics JSON' : 'Export Diagnostics'}</span>
        </Button>
      </div>

      {#if diagnosticsData}
        <div class="mt-3 rounded-lg bg-secondary/40 border border-border/40 p-3 text-meta font-mono text-muted-foreground space-y-1 overflow-x-auto max-h-48 overflow-y-auto">
          <div><span class="text-foreground font-semibold">Zenith:</span> {diagnosticsData.app_version} ({diagnosticsData.arch})</div>
          <div><span class="text-foreground font-semibold">OS:</span> {diagnosticsData.os_version}</div>
          <div><span class="text-foreground font-semibold">Log:</span> {diagnosticsData.log_path}</div>
          {#if diagnosticsData.recent_errors.length > 0}
            <div class="pt-2 text-destructive font-semibold">Recent Errors ({diagnosticsData.recent_errors.length}):</div>
            {#each diagnosticsData.recent_errors as err}
              <div class="text-destructive/80 truncate">{err}</div>
            {/each}
          {:else}
            <div class="pt-1 text-success/80">No recent errors logged.</div>
          {/if}
        </div>
      {/if}
    </Card>
  </div>

  <!-- About -->
  <div class="space-y-3 pt-2">
    <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
      About Zenith
    </h3>
    <Card class="p-4 bg-card/70 text-xs space-y-2">
      <div class="flex items-center justify-between">
        <span class="font-medium text-foreground">Zenith Developer System Manager</span>
        <Badge variant="outline" class="font-mono">{formatVersion(APP_VERSION)}</Badge>
      </div>
      <p class="text-muted-foreground leading-relaxed">
        Zenith is an ultra-lightweight open-source utility designed to safely manage AI caches, developer build artifacts, Docker storage, local LLMs, memory pressure, and keep-awake power assertions.
      </p>
      <div class="pt-2 text-meta text-muted-foreground font-mono">
        Built with Tauri 2 + Svelte 5 + Rust. Zero analytics, zero cloud, 100% local.
      </div>
    </Card>
  </div>
</div>
