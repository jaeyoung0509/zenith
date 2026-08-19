<script lang="ts">
  import { onMount } from 'svelte';
  import type { Component } from 'svelte';
  import { settingsStore } from './lib/stores/settings.svelte';
  import { isTauri } from './lib/utils/tauri';
  import { isQuickPanelDismissShortcut } from './lib/utils/quickPanel';

  let currentView = $state<'dashboard' | 'quick'>('dashboard');
  let ActiveComponent = $state<Component<any> | null>(null);

  async function loadViewComponent(view: 'dashboard' | 'quick') {
    if (view === 'quick') {
      const module: any = await import('./routes/quick/QuickPanel.svelte');
      ActiveComponent = module.default ?? module;
    } else {
      const module: any = await import('./routes/dashboard/Dashboard.svelte');
      ActiveComponent = module.default ?? module;
    }
  }

  onMount(() => {
    settingsStore.load();

    let disposed = false;

    const checkRoute = () => {
      const hash = window.location.hash;
      const isQuickWindow = window.location.search.includes('quick') || hash.includes('quick');
      const nextView = isQuickWindow ? 'quick' : 'dashboard';
      if (currentView !== nextView || !ActiveComponent) {
        currentView = nextView;
        void loadViewComponent(nextView);
      }
    };

    checkRoute();
    window.addEventListener('hashchange', checkRoute);

    const closeOnCommandW = (event: KeyboardEvent) => {
      const shouldClose = event.metaKey && event.key.toLowerCase() === 'w';
      const shouldDismissQuick = currentView === 'quick' && isQuickPanelDismissShortcut(event.key, event.metaKey);
      if (!isTauri || (!shouldClose && !shouldDismissQuick)) return;

      event.preventDefault();
      void import('@tauri-apps/api/webviewWindow').then(({ getCurrentWebviewWindow }) => {
        return getCurrentWebviewWindow().hide();
      });
    };
    window.addEventListener('keydown', closeOnCommandW, true);

    // Check Tauri window label first
    void (async () => {
      try {
        const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
        const win = getCurrentWebviewWindow();
        if (!disposed && win.label === 'quick') {
          currentView = 'quick';
          void loadViewComponent('quick');
        } else if (!disposed && !ActiveComponent) {
          void loadViewComponent('dashboard');
        }
      } catch {
        if (!disposed && !ActiveComponent) {
          void loadViewComponent(currentView);
        }
      }
    })();

    return () => {
      disposed = true;
      window.removeEventListener('hashchange', checkRoute);
      window.removeEventListener('keydown', closeOnCommandW, true);
    };
  });
</script>

{#if ActiveComponent}
  <ActiveComponent />
{/if}
