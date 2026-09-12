<script lang="ts">
  import { onMount } from 'svelte';
  import type { Component } from 'svelte';

  import { settingsStore } from './lib/stores/settings.svelte';
  import { platformCapabilitiesStore } from './lib/stores/platformCapabilities.svelte';
  import { platformContextStore } from './lib/stores/platformContext.svelte';
  import { isTauri } from './lib/utils/tauri';
  import {
    isAcceleratorPressed,
    isQuickPanelDismissShortcut,
    platformAccelerator,
  } from './lib/utils/quickPanel';

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
    void platformCapabilitiesStore.load();
    void platformContextStore.load();

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
      if (isTauri()) {
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
      // The dashboard close chord keeps its historical behavior; the quick
      // panel dismissal follows the platform's primary accelerator so Ctrl+W
      // also works on Windows and Linux.
      const shouldClose = event.metaKey && event.key.toLowerCase() === 'w';
      const accelerator = platformAccelerator(platformContextStore.context?.primary_accelerator);
      const shouldDismissQuick =
        currentView === 'quick' &&
        isQuickPanelDismissShortcut(event.key, isAcceleratorPressed(event, accelerator));

      if (!isTauri() || (!shouldClose && !shouldDismissQuick)) {
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
      settingsStore.cleanup();
    };
  });
</script>

{#if ActiveComponent}
  <ActiveComponent />
{/if}
