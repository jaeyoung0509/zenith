import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import Dashboard from '../routes/dashboard/Dashboard.svelte';
import { scanStore } from '../lib/stores/scan.svelte';

beforeEach(() => {
  scanStore.lastScan = {
    scan_id: 'sidebar-test',
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
});
