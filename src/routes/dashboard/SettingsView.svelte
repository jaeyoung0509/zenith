<script lang="ts">
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import type { AiProviderId, DashboardTab, QuickPanelSection } from '../../lib/models/types';
  import Card from '../../lib/components/Card.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import { Settings, Sparkles, Moon, Sun, Monitor, ChevronUp, ChevronDown, PanelTop, LayoutList } from 'lucide-svelte';

  const tabOptions: { id: DashboardTab; label: string; description: string }[] = [
    { id: 'disk', label: 'Disks', description: 'Disk volumes, mount points, and storage health.' },
    { id: 'storage', label: 'Storage', description: 'System-wide reclaimable developer and AI caches.' },
    { id: 'docker', label: 'Containers', description: 'Docker images, build cache, stopped containers, and volumes.' },
    { id: 'models', label: 'Local Models', description: 'Ollama, HuggingFace, LM Studio, and Apple MLX models.' },
    { id: 'memory', label: 'Memory', description: 'Memory pressure, top processes, and resource guard.' },
    { id: 'usage', label: 'AI Usage', description: 'OAuth coding agent limits and local token insights.' },
    { id: 'awake', label: 'Keep Awake', description: 'Prevent system and display sleep rules.' },
  ];

  const sectionOptions: { id: QuickPanelSection; label: string; description: string }[] = [
    { id: 'storage', label: 'Storage', description: 'Primary disk capacity and usage.' },
    { id: 'cleanup', label: 'Quick Clean', description: 'Safe reclaimable storage and clean action.' },
    { id: 'ai_usage', label: 'AI Usage', description: 'Connected provider limits and local activity.' },
    { id: 'categories', label: 'Storage Categories', description: 'AI, developer, container, model, and system totals.' },
    { id: 'memory', label: 'Memory', description: 'Memory pressure and current usage.' },
  ];
  const providerOptions: { id: AiProviderId; label: string }[] = [
    { id: 'codex', label: 'Codex' },
    { id: 'claude', label: 'Claude Code' },
    { id: 'opencode', label: 'OpenCode' },
    { id: 'openrouter', label: 'OpenRouter' },
    { id: 'antigravity', label: 'Antigravity' },
  ];

  let settings = $derived(settingsStore.settings);

  function handleToggle(key: keyof typeof settings) {
    if (typeof settings[key] === 'boolean') {
      settingsStore.save({ [key]: !settings[key] });
    }
  }

  function handleTheme(theme: string) {
    settingsStore.save({ theme });
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

  function orderedProviders() {
    const selected = settings.quick_panel_ai_providers
      .map((id) => providerOptions.find((option) => option.id === id))
      .filter((option): option is (typeof providerOptions)[number] => Boolean(option));
    return [...selected, ...providerOptions.filter((option) => !settings.quick_panel_ai_providers.includes(option.id))];
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
    <div class="rounded-xl border border-red-500/20 bg-red-500/5 px-4 py-3 text-xs text-red-500">
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
          <div class="text-[11px] text-muted-foreground">Autostart is not enabled in this build.</div>
        </div>
        <label class="relative inline-flex items-center cursor-not-allowed opacity-60">
          <input
            type="checkbox"
            checked={settings.launch_at_login}
            disabled
            class="sr-only peer"
          />
          <div
            class="w-9 h-5 bg-secondary peer-focus:outline-none peer-focus:ring-2 peer-focus:ring-emerald-500/30 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-emerald-500"
          ></div>
        </label>
      </div>
    </Card>
  </div>

  <!-- Dashboard Navigation Customization -->
  <div class="space-y-3">
    <div>
      <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
        Dashboard Navigation Menu
      </h3>
      <p class="text-[11px] text-muted-foreground mt-1">
        Customize the tabs displayed in the left sidebar and their ordering.
      </p>
    </div>
    <Card class="p-4 bg-card/70 space-y-3">
      <div class="flex items-center gap-2 text-xs font-medium text-foreground pb-1">
        <LayoutList size={14} /> Sidebar Menu Order
      </div>
      {#each orderedDashboardTabs() as tabOption}
        {@const enabled = (settings.dashboard_tabs ?? []).includes(tabOption.id)}
        {@const index = (settings.dashboard_tabs ?? []).indexOf(tabOption.id)}
        <div class="flex items-center gap-3 rounded-lg border border-border/60 px-3 py-2.5">
          <input
            type="checkbox"
            checked={enabled}
            disabled={enabled && (settings.dashboard_tabs ?? []).length === 1}
            onchange={() => settingsStore.toggleDashboardTab(tabOption.id)}
            aria-label={`Show ${tabOption.label} in sidebar`}
            class="h-3.5 w-3.5 accent-primary cursor-pointer"
          />
          <div class="min-w-0 flex-1">
            <div class="text-xs font-medium text-foreground">{tabOption.label}</div>
            <div class="text-[10px] text-muted-foreground">{tabOption.description}</div>
          </div>
          {#if enabled}
            <button
              type="button"
              class="p-1 text-muted-foreground hover:text-foreground disabled:opacity-30 transition-colors"
              disabled={index === 0}
              onclick={() => settingsStore.moveDashboardTab(tabOption.id, -1)}
              aria-label={`Move ${tabOption.label} up`}
            >
              <ChevronUp size={14} />
            </button>
            <button
              type="button"
              class="p-1 text-muted-foreground hover:text-foreground disabled:opacity-30 transition-colors"
              disabled={index === (settings.dashboard_tabs ?? []).length - 1}
              onclick={() => settingsStore.moveDashboardTab(tabOption.id, 1)}
              aria-label={`Move ${tabOption.label} down`}
            >
              <ChevronDown size={14} />
            </button>
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
      <p class="text-[11px] text-muted-foreground mt-1">
        Choose what appears below the menu bar icon and set the display priority.
      </p>
    </div>
    <Card class="p-4 bg-card/70 space-y-5">
      <div class="space-y-2">
        <div class="flex items-center gap-2 text-xs font-medium text-foreground">
          <PanelTop size={14} /> Sections
        </div>
        {#each orderedSections() as option}
          {@const enabled = settings.quick_panel_sections.includes(option.id)}
          {@const index = settings.quick_panel_sections.indexOf(option.id)}
          <div class="flex items-center gap-3 rounded-lg border border-border/60 px-3 py-2.5">
            <input
              type="checkbox"
              checked={enabled}
              disabled={enabled && settings.quick_panel_sections.length === 1}
              onchange={() => settingsStore.toggleQuickPanelSection(option.id)}
              aria-label={`Show ${option.label} in quick panel`}
              class="h-3.5 w-3.5 accent-primary cursor-pointer"
            />
            <div class="min-w-0 flex-1">
              <div class="text-xs font-medium text-foreground">{option.label}</div>
              <div class="text-[10px] text-muted-foreground">{option.description}</div>
            </div>
            {#if enabled}
              <button type="button" class="p-1 text-muted-foreground hover:text-foreground disabled:opacity-30 transition-colors" disabled={index === 0} onclick={() => settingsStore.moveQuickPanelSection(option.id, -1)} aria-label={`Move ${option.label} up`}><ChevronUp size={14} /></button>
              <button type="button" class="p-1 text-muted-foreground hover:text-foreground disabled:opacity-30 transition-colors" disabled={index === settings.quick_panel_sections.length - 1} onclick={() => settingsStore.moveQuickPanelSection(option.id, 1)} aria-label={`Move ${option.label} down`}><ChevronDown size={14} /></button>
            {/if}
          </div>
        {/each}
      </div>

      <div class="space-y-2 pt-4 border-t border-border/60">
        <div class="flex items-center gap-2 text-xs font-medium text-foreground">
          <Sparkles size={14} /> AI Provider Priority
        </div>
        <p class="text-[10px] text-muted-foreground">Only enabled providers are displayed in the quick panel, in this order.</p>
        {#each orderedProviders() as provider}
          {@const enabled = settings.quick_panel_ai_providers.includes(provider.id)}
          {@const index = settings.quick_panel_ai_providers.indexOf(provider.id)}
          <div class="flex items-center gap-3 rounded-lg border border-border/60 px-3 py-2">
            <input type="checkbox" checked={enabled} onchange={() => settingsStore.toggleQuickPanelProvider(provider.id)} aria-label={`Show ${provider.label} usage`} class="h-3.5 w-3.5 accent-primary cursor-pointer" />
            <span class="flex-1 text-xs font-medium text-foreground">{provider.label}</span>
            {#if enabled}
              <button type="button" class="p-1 text-muted-foreground hover:text-foreground disabled:opacity-30 transition-colors" disabled={index === 0} onclick={() => settingsStore.moveQuickPanelProvider(provider.id, -1)} aria-label={`Move ${provider.label} up`}><ChevronUp size={14} /></button>
              <button type="button" class="p-1 text-muted-foreground hover:text-foreground disabled:opacity-30 transition-colors" disabled={index === settings.quick_panel_ai_providers.length - 1} onclick={() => settingsStore.moveQuickPanelProvider(provider.id, 1)} aria-label={`Move ${provider.label} down`}><ChevronDown size={14} /></button>
            {/if}
          </div>
        {/each}
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
          <div class="text-[11px] text-muted-foreground">Claude Code, Cursor, Gemini CLI, Codex, Aider.</div>
        </div>
        <label class="relative inline-flex items-center cursor-pointer">
          <input
            type="checkbox"
            checked={settings.clean_ai_tools}
            onchange={() => handleToggle('clean_ai_tools')}
            class="sr-only peer"
          />
          <div
            class="w-9 h-5 bg-secondary peer-focus:outline-none peer-focus:ring-2 peer-focus:ring-emerald-500/30 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-emerald-500"
          ></div>
        </label>
      </div>

      <!-- Developer Tools -->
      <div class="flex items-center justify-between text-xs pt-3">
        <div>
          <div class="font-medium text-foreground">Developer Compilers & Package Managers</div>
          <div class="text-[11px] text-muted-foreground">Go build, Cargo cache, npm, pnpm, uv, Xcode DerivedData.</div>
        </div>
        <label class="relative inline-flex items-center cursor-pointer">
          <input
            type="checkbox"
            checked={settings.clean_developer_tools}
            onchange={() => handleToggle('clean_developer_tools')}
            class="sr-only peer"
          />
          <div
            class="w-9 h-5 bg-secondary peer-focus:outline-none peer-focus:ring-2 peer-focus:ring-emerald-500/30 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-emerald-500"
          ></div>
        </label>
      </div>

      <!-- Docker -->
      <div class="flex items-center justify-between text-xs pt-3">
        <div>
          <div class="font-medium text-foreground">Docker Dangling Images & BuildKit Cache</div>
          <div class="text-[11px] text-muted-foreground">Clean safe Docker cache layers via official Docker CLI.</div>
        </div>
        <label class="relative inline-flex items-center cursor-pointer">
          <input
            type="checkbox"
            checked={settings.clean_docker}
            onchange={() => handleToggle('clean_docker')}
            class="sr-only peer"
          />
          <div
            class="w-9 h-5 bg-secondary peer-focus:outline-none peer-focus:ring-2 peer-focus:ring-emerald-500/30 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-emerald-500"
          ></div>
        </label>
      </div>

      <!-- Local Models -->
      <div class="flex items-center justify-between text-xs pt-3">
        <div>
          <div class="font-medium text-foreground">Local Models (Ollama / HuggingFace)</div>
          <div class="text-[11px] text-muted-foreground">Always off by default to protect stateful weights.</div>
        </div>
        <label class="relative inline-flex items-center cursor-pointer">
          <input
            type="checkbox"
            checked={settings.clean_local_models}
            onchange={() => handleToggle('clean_local_models')}
            class="sr-only peer"
          />
          <div
            class="w-9 h-5 bg-secondary peer-focus:outline-none peer-focus:ring-2 peer-focus:ring-emerald-500/30 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-emerald-500"
          ></div>
        </label>
      </div>
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
          class="flex flex-col items-center justify-center p-3 rounded-lg border text-xs gap-2 transition-all {settings.theme ===
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
          class="flex flex-col items-center justify-center p-3 rounded-lg border text-xs gap-2 transition-all {settings.theme ===
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
          class="flex flex-col items-center justify-center p-3 rounded-lg border text-xs gap-2 transition-all {settings.theme ===
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

  <!-- About -->
  <div class="space-y-3 pt-2">
    <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
      About Zenith
    </h3>
    <Card class="p-4 bg-card/70 text-xs space-y-2">
      <div class="flex items-center justify-between">
        <span class="font-medium text-foreground">Zenith macOS Developer System Manager</span>
        <Badge variant="outline" class="font-mono">v0.1.0</Badge>
      </div>
      <p class="text-muted-foreground leading-relaxed">
        Zenith is an ultra-lightweight open-source utility designed to safely manage AI caches, developer build artifacts, Docker storage, local LLMs, memory pressure, and keep-awake power assertions.
      </p>
      <div class="pt-2 text-[11px] text-muted-foreground font-mono">
        Built with Tauri 2 + Svelte 5 + Rust. Zero analytics, zero cloud, 100% local.
      </div>
    </Card>
  </div>
</div>
