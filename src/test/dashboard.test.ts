import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { render } from 'svelte/server';
import Dashboard from '../routes/dashboard/Dashboard.svelte';
import { scanStore } from '../lib/stores/scan.svelte';
import { platformCapabilitiesStore } from '../lib/stores/platformCapabilities.svelte';
import { platformContextStore } from '../lib/stores/platformContext.svelte';
import { goldenCapabilitiesByPlatform } from '../lib/models/platformCapabilities';
import { mockApi } from '../lib/api/mock';

beforeEach(() => {
  scanStore.lastScan = {
    scan_id: 'sidebar-test',
    valid_for_seconds: 300,
    started_at: Date.now() - 1000,
    finished_at: Date.now(),
    categories: [
      {
        category: 'ai',
        display_name: 'AI Tools',
        items: [
          {
            id: 'sidebar-safe',
            signature_id: 'sidebar.safe',
            name: 'Sidebar test cache',
            category: 'ai',
            risk: 'safe',
            path: '/tmp/sidebar-test',
            size: { logical: 14 * 1024 * 1024, allocated: 14 * 1024 * 1024 },
            file_count: 1,
            description: 'Dashboard sidebar fixture',
            is_selected: true,
            last_modified: Date.now(),
            exists: true,
          },
        ],
        total_bytes: 14 * 1024 * 1024,
        safe_bytes: 14 * 1024 * 1024,
        rebuild_bytes: 0,
        manual_bytes: 0,
      },
    ],
    total_bytes: 14 * 1024 * 1024,
    safe_bytes: 14 * 1024 * 1024,
    rebuild_bytes: 0,
    manual_bytes: 0,
  };
  scanStore.selectedMap = { 'sidebar-safe': true };
});

afterEach(() => {
  scanStore.lastScan = null;
  scanStore.selectedMap = {};
  platformCapabilitiesStore.reset();
  platformContextStore.reset();
});

describe('Dashboard sidebar affordances', () => {
  it('keeps the collapse control labelled and the Storage status visually quiet', () => {
    const rendered = render(Dashboard);

    expect(rendered.body).toContain('aria-label="Collapse sidebar"');
    expect(rendered.body).toContain('title="Collapse sidebar"');
    expect(rendered.body).toContain('rounded-md border border-transparent');
    expect(rendered.body).toContain('text-success/85');
    expect(rendered.body).toContain('14 MB');
  });

  it('exposes Development Servers as its own dashboard route', () => {
    const rendered = render(Dashboard);
    const memorySource = readFileSync(
      new URL('../routes/dashboard/MemoryView.svelte', import.meta.url),
      'utf8'
    );

    expect(rendered.body).toContain('Dev Servers');
    expect(memorySource).not.toContain('developmentPortsStore');
    expect(memorySource).not.toContain('Development Servers Section');
  });

  it('exposes the consolidated AI Activity dashboard route', () => {
    const rendered = render(Dashboard);
    expect(rendered.body).toContain('AI Activity');
  });
});

describe('Dashboard platform chrome', () => {
  it('reserves the overlay top band only when the backend reports one', async () => {
    platformCapabilitiesStore.capabilities = goldenCapabilitiesByPlatform.macos;

    platformContextStore.context = await mockApi.getPlatformContext();
    const overlay = render(Dashboard);
    expect(overlay.body).toContain('titlebar-drag-region absolute top-0 left-0 right-0 h-7 z-30');
    expect(overlay.body).toContain('pt-9');
    expect(overlay.body).toContain('pt-10');

    platformContextStore.context = {
      ...(await mockApi.getPlatformContext()),
      platform: 'windows',
      overlay_title_bar: false,
      native_caption_bar: true,
    };
    const nativeCaption = render(Dashboard);
    // A native caption bar must neither reserve the band nor expose a drag strip.
    expect(nativeCaption.body).not.toContain('titlebar-drag-region');
    expect(nativeCaption.body).not.toContain('pt-9');
    expect(nativeCaption.body).not.toContain('pt-10');
  });

  it('renders a retryable capability failure instead of an unsupported platform', () => {
    platformCapabilitiesStore.capabilities = null;
    platformCapabilitiesStore.error = 'IPC unavailable';

    const failed = render(Dashboard);

    expect(failed.body).toContain('Platform capabilities unavailable');
    expect(failed.body).toContain('IPC unavailable');
    expect(failed.body).toContain('Retry');
    expect(failed.body).not.toContain('Loading platform capabilities');

    platformCapabilitiesStore.capabilities = goldenCapabilitiesByPlatform.windows;
    platformCapabilitiesStore.error = null;

    const unsupported = render(Dashboard);

    expect(unsupported.body).not.toContain('Platform capabilities unavailable');
    expect(unsupported.body).not.toContain('Loading platform capabilities');
  });
});
