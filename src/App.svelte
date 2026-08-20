<script lang="ts">
  import { onMount } from 'svelte';
  import type { Component } from 'svelte';

  import { settingsStore } from './lib/stores/settings.svelte';
  import { isTauri } from './lib/utils/tauri';
  import { isQuickPanelDismissShortcut } from './lib/utils/quickPanel';

  type View = 'dashboard' | 'quick';

  let currentView = $state<View | null>(null);
  let ActiveComponent = $state<Component<any> | null>(null);

  function browserView(): View {
    const hash = window.location.hash;
    const search = window.location.search;

    return search.includes('quick') || hash.includes('quick')
      ? 'quick'
      : 'dashboard';
  }

  async function importView(view: View): Promise<Component<any>> {
    if (view === 'quick') {
      const module: any = await import('./routes/quick/QuickPanel.svelte');
      return module.default ?? module;
    }

    const module: any = await import('./routes/dashboard/Dashboard.svelte');
    return module.default ?? module;
  }

  onMount(() => {
    let disposed = false;
    let loadGeneration = 0;
    let removeHashListener: (() => void) | undefined;

    settingsStore.load();

    async function activateView(view: View) {
      const generation = ++loadGeneration;

      // Start chunk download
      const component = await importView(view);

      // Discard stale import completion
      if (disposed || generation !== loadGeneration) return;

      currentView = view;
      ActiveComponent = component;
    }

    async function resolveInitialView() {
      if (isTauri) {
        try {
          const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
          if (disposed) return;
          const win = getCurrentWebviewWindow();
          const view: View = win.label === 'quick' ? 'quick' : 'dashboard';
          await activateView(view);
          return;
        } catch {
          // Fallback if window API fails
        }
      }

      // Browser preview routing with query/hash support
      await activateView(browserView());

      const onHashChange = () => {
        void activateView(browserView());
      };

      window.addEventListener('hashchange', onHashChange);

      removeHashListener = () => {
        window.removeEventListener('hashchange', onHashChange);
      };
    }

    void resolveInitialView();

    const closeOnCommandW = (event: KeyboardEvent) => {
      const shouldClose = event.metaKey && event.key.toLowerCase() === 'w';
      const shouldDismissQuick =
        currentView === 'quick' && isQuickPanelDismissShortcut(event.key, event.metaKey);

      if (!isTauri || (!shouldClose && !shouldDismissQuick)) {
        return;
      }

      event.preventDefault();

      void import('@tauri-apps/api/webviewWindow').then(({ getCurrentWebviewWindow }) => {
        return getCurrentWebviewWindow().hide();
      });
    };

    window.addEventListener('keydown', closeOnCommandW, true);

    return () => {
      disposed = true;
      loadGeneration++;
      removeHashListener?.();
      window.removeEventListener('keydown', closeOnCommandW, true);
    };
  });
</script>

{#if ActiveComponent}
  <ActiveComponent />
{/if}
