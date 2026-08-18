<script lang="ts">
  import { onMount } from 'svelte';
  import QuickPanel from './routes/quick/QuickPanel.svelte';
  import Dashboard from './routes/dashboard/Dashboard.svelte';
  import { settingsStore } from './lib/stores/settings.svelte';

  let currentView = $state<'dashboard' | 'quick'>('dashboard');

  onMount(() => {
    settingsStore.load();

    let disposed = false;

    const checkRoute = () => {
      const hash = window.location.hash;
      const isQuickWindow = window.location.search.includes('quick') || hash.includes('quick');
      currentView = isQuickWindow ? 'quick' : 'dashboard';
    };

    checkRoute();
    window.addEventListener('hashchange', checkRoute);

    // Check Tauri window label first
    void (async () => {
      try {
        const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
        const win = getCurrentWebviewWindow();
        if (!disposed && win.label === 'quick') {
          currentView = 'quick';
        }
      } catch {
        // Browser or mock fallback
      }
    })();

    return () => {
      disposed = true;
      window.removeEventListener('hashchange', checkRoute);
    };
  });
</script>

{#if currentView === 'quick'}
  <QuickPanel />
{:else}
  <Dashboard />
{/if}
