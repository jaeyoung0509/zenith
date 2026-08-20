import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render } from 'svelte/server';
import StorageView from '../routes/dashboard/StorageView.svelte';
import { scanStore } from '../lib/stores/scan.svelte';
import type { CategoryResult } from '../lib/models/types';

describe('StorageView CTA and responsive toolbar layout', () => {
  beforeEach(() => {
    scanStore.lastScan = null;
    scanStore.selectedMap = {};
    scanStore.isScanning = false;
    scanStore.isCleaning = false;
  });

  it('renders "Clean Safely" for safe-only selections without duplicating byte count in CTA text', () => {
    const mockCategory: CategoryResult = {
      category: 'ai',
      display_name: 'AI Tools',
      items: [
        {
          id: 'item-1',
          signature_id: 'sig.ai.1',
          name: 'Claude Cache',
          category: 'ai',
          risk: 'safe',
          path: '/tmp/claude',
          size: { logical: 1024 * 1024 * 100, allocated: 1024 * 1024 * 100 },
          file_count: 50,
          description: 'Claude session caches',
          is_selected: true,
          last_modified: Date.now(),
          exists: true,
        },
      ],
      total_bytes: 1024 * 1024 * 100,
      safe_bytes: 1024 * 1024 * 100,
      rebuild_bytes: 0,
      manual_bytes: 0,
    };

    scanStore.lastScan = {
      scan_id: 'scan-1',
      started_at: Date.now() - 1000,
      finished_at: Date.now(),
      categories: [mockCategory],
      total_bytes: 1024 * 1024 * 100,
      safe_bytes: 1024 * 1024 * 100,
      rebuild_bytes: 0,
      manual_bytes: 0,
    };
    scanStore.selectedMap = { 'item-1': true };

    const rendered = render(StorageView, {
      props: {
        onSelectCategory: vi.fn(),
      },
    });

    expect(rendered.body).toContain('Clean Safely');
    expect(rendered.body).not.toContain('Review & Clean');
    // Ensure the CTA button doesn't contain duplicated byte label
    expect(rendered.body).toContain('✓ 100 MB Safe');
    // Ensure responsive toolbar classes for 960x660 baseline
    expect(rendered.body).toContain('flex flex-col md:flex-row md:items-center justify-between gap-3');
    expect(rendered.body).toContain('aria-label="Open macOS Disk Utility"');
  });

  it('renders "Review & Clean" when rebuildable items are selected', () => {
    const mockCategory: CategoryResult = {
      category: 'developer',
      display_name: 'Developer Caches',
      items: [
        {
          id: 'item-rebuild',
          signature_id: 'sig.dev.cargo',
          name: 'Cargo Target',
          category: 'developer',
          risk: 'rebuild',
          path: '/tmp/target',
          size: { logical: 1024 * 1024 * 500, allocated: 1024 * 1024 * 500 },
          file_count: 120,
          description: 'Rebuildable build artifacts',
          is_selected: true,
          last_modified: Date.now(),
          exists: true,
        },
      ],
      total_bytes: 1024 * 1024 * 500,
      safe_bytes: 0,
      rebuild_bytes: 1024 * 1024 * 500,
      manual_bytes: 0,
    };

    scanStore.lastScan = {
      scan_id: 'scan-2',
      started_at: Date.now() - 1000,
      finished_at: Date.now(),
      categories: [mockCategory],
      total_bytes: 1024 * 1024 * 500,
      safe_bytes: 0,
      rebuild_bytes: 1024 * 1024 * 500,
      manual_bytes: 0,
    };
    scanStore.selectedMap = { 'item-rebuild': true };
    expect(scanStore.rebuildSelectedBytes).toBe(1024 * 1024 * 500);

    const rendered = render(StorageView, {
      props: {
        onSelectCategory: vi.fn(),
      },
    });

    expect(rendered.body).toContain('Review &amp; Clean');
    expect(rendered.body).toContain('↻ 500 MB Rebuildable');
  });
});
