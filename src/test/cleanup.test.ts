import { describe, expect, it } from 'vitest';
import type { ScanItem } from '../lib/models/types';
import { filterAndSortCleanupItems } from '../lib/utils/cleanup';
import { cleanOutcome } from '../lib/utils/cleanResult';

function item(overrides: Partial<ScanItem>): ScanItem {
  return {
    id: 'cache',
    signature_id: 'dev.cache',
    name: 'Cache',
    category: 'developer',
    risk: 'safe',
    path: '/tmp/cache',
    size: { logical: 1, allocated: 1 },
    file_count: 1,
    description: 'Generated cache',
    is_selected: false,
    last_modified: null,
    exists: true,
    ...overrides,
  };
}

describe('filterAndSortCleanupItems', () => {
  it('hides nonexistent and empty signature locations', () => {
    const result = filterAndSortCleanupItems(
      [
        item({ id: 'real' }),
        item({ id: 'missing', exists: false }),
        item({ id: 'empty', size: { logical: 0, allocated: 0 } }),
      ],
      'all',
      '',
      'size'
    );
    expect(result.map((entry) => entry.id)).toEqual(['real']);
  });

  it('orders by reclaimable bytes and keeps rebuild items opt-in', () => {
    const result = filterAndSortCleanupItems(
      [
        item({ id: 'small', size: { logical: 10, allocated: 10 } }),
        item({ id: 'large', risk: 'rebuild', size: { logical: 100, allocated: 100 } }),
      ],
      'all',
      '',
      'size'
    );
    expect(result.map((entry) => entry.id)).toEqual(['large', 'small']);
    expect(result[0].is_selected).toBe(false);
  });

  it('filters by risk and searchable metadata', () => {
    const result = filterAndSortCleanupItems(
      [
        item({ id: 'cargo', name: 'Cargo Registry', risk: 'rebuild', path: '/cargo/cache' }),
        item({ id: 'npm', name: 'npm Cache', path: '/npm/cache' }),
        item({ id: 'ollama', name: 'Ollama Blobs', risk: 'manual', path: '/ollama/models' }),
      ],
      'manual',
      'ollama',
      'name'
    );
    expect(result.map((entry) => entry.id)).toEqual(['ollama']);
  });
});

describe('cleanOutcome', () => {
  const item = (status: 'success' | 'partial' | 'failed', success: boolean, error_message: string | null = null) => ({
    item_id: status,
    name: status,
    path: `/tmp/${status}`,
    status,
    success,
    bytes_reclaimed: success ? 10 : 0,
    failure_reason: null,
    error_message,
  });

  it('reports a total failure when every item fails', () => {
    expect(cleanOutcome({ items: [item('failed', false)] })).toBe('failed');
  });

  it('reports partial cleanup when successful and failed items are mixed', () => {
    expect(cleanOutcome({ items: [item('success', true), item('failed', false)] })).toBe('partial');
    expect(cleanOutcome({ items: [item('partial', true, 'one file was locked')] })).toBe('partial');
  });

  it('reports complete cleanup only when every item succeeds cleanly', () => {
    expect(cleanOutcome({ items: [item('success', true), item('success', true)] })).toBe('success');
    expect(cleanOutcome({ items: [] })).toBe('failed');
  });
});

describe('quick clean eligibility and predicate consistency', () => {
  it('strictly selects only safe risk items and honors category toggle settings', async () => {
    const { scanStore } = await import('../lib/stores/scan.svelte');

    const mockScan = {
      scan_id: 'scan-123',
      started_at: 1000,
      created_at: 1000,
      finished_at: 1005,
      total_bytes: 4000,
      safe_bytes: 1500,
      rebuild_bytes: 2000,
      manual_bytes: 0,
      categories: [
        {
          category: 'ai' as const,
          display_name: 'AI Tools',
          total_bytes: 1000,
          safe_bytes: 1000,
          rebuild_bytes: 2000,
          manual_bytes: 0,
          items: [
            item({ id: 'ai-safe', category: 'ai', risk: 'safe', size: { logical: 1000, allocated: 1000 } }),
            item({ id: 'ai-rebuild', category: 'ai', risk: 'rebuild', size: { logical: 2000, allocated: 2000 } }),
          ],
        },
        {
          category: 'developer' as const,
          display_name: 'Developer Tools',
          total_bytes: 500,
          safe_bytes: 500,
          rebuild_bytes: 0,
          manual_bytes: 0,
          items: [
            item({ id: 'dev-safe', category: 'developer', risk: 'safe', size: { logical: 500, allocated: 500 } }),
          ],
        },
      ],
    };

    scanStore.lastScan = mockScan;

    const allEnabledSettings = {
      launch_at_login: false,
      clean_ai_tools: true,
      clean_developer_tools: true,
      clean_docker: true,
      clean_local_models: false,
      include_rebuild_caches: true, // Even if include_rebuild_caches is true, Quick Clean must be Safe only!
      intensive_cleanup: true,
      theme: 'system',
      excluded_signatures: [],
      quick_panel_sections: ['cleanup', 'storage', 'memory', 'ai_usage'] as any,
      quick_panel_ai_providers: [],
      dashboard_tabs: ['storage'] as any,
      dashboard_tabs_revision: 1,
      sidebar_collapsed: false,
      awake_rules: [],
      ai_control: {},
    };

    // Calculate bytes
    const cleanableBytes = scanStore.quickCleanableBytes(allEnabledSettings);
    expect(cleanableBytes).toBe(1500); // 1000 (ai-safe) + 500 (dev-safe)

    // Select defaults
    scanStore.selectQuickCleanDefaults(allEnabledSettings);
    expect(scanStore.selectedMap['ai-safe']).toBe(true);
    expect(scanStore.selectedMap['dev-safe']).toBe(true);
    expect(scanStore.selectedMap['ai-rebuild']).toBe(false); // rebuild NEVER selected in quick clean

    // If AI category disabled in settings
    const aiDisabledSettings = { ...allEnabledSettings, clean_ai_tools: false };
    expect(scanStore.quickCleanableBytes(aiDisabledSettings)).toBe(500);

    scanStore.selectQuickCleanDefaults(aiDisabledSettings);
    expect(scanStore.selectedMap['ai-safe']).toBe(false);
    expect(scanStore.selectedMap['dev-safe']).toBe(true);
  });
});
