<script lang="ts">
  import { settingsStore } from '../../lib/stores/settings.svelte';
  import Card from '../../lib/components/Card.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import { Settings, Shield, Sparkles, Moon, Sun, Monitor, Laptop } from 'lucide-svelte';

  let settings = $derived(settingsStore.settings);

  function handleToggle(key: keyof typeof settings) {
    if (typeof settings[key] === 'boolean') {
      settingsStore.save({ [key]: !settings[key] });
    }
  }

  function handleTheme(theme: string) {
    settingsStore.save({ theme });
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

  <!-- General Preferences -->
  <div class="space-y-3">
    <h3 class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
      General
    </h3>
    <Card class="p-4 space-y-4 bg-card/70">
      <div class="flex items-center justify-between text-xs">
        <div>
          <div class="font-medium text-foreground">Launch Zenith at login</div>
          <div class="text-[11px] text-muted-foreground">Start menu bar agent automatically on system startup.</div>
        </div>
        <label class="relative inline-flex items-center cursor-pointer">
          <input
            type="checkbox"
            checked={settings.launch_at_login}
            onchange={() => handleToggle('launch_at_login')}
            class="sr-only peer"
          />
          <div
            class="w-9 h-5 bg-secondary peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-primary"
          ></div>
        </label>
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
            class="w-9 h-5 bg-secondary peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-primary"
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
            class="w-9 h-5 bg-secondary peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-primary"
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
            class="w-9 h-5 bg-secondary peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-primary"
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
            class="w-9 h-5 bg-secondary peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-primary"
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
