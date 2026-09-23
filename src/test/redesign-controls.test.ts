import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import SegmentedTabs from '../lib/components/SegmentedTabs.svelte';
import SelectionToolbar from '../lib/components/SelectionToolbar.svelte';
import InlineNotice from '../lib/components/InlineNotice.svelte';
import CleanupReviewDialog from '../lib/components/CleanupReviewDialog.svelte';
import type { CleanupMode, PlanPreview, ScanItem } from '../lib/models/types';
import { normalizeDashboardTab } from '../lib/utils/dashboardNavigation';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const item = {
  id: 'fixture', signature_id: 'fixture', name: 'Build cache', category: 'developer',
  risk: 'rebuild', size: { logical: 1024, allocated: 1024 }, path: '/fixture',
  file_count: 1, description: '', is_selected: true, exists: true, last_modified: null,
} as ScanItem;

function planFor(items: ScanItem[], mode: CleanupMode = 'trash'): PlanPreview {
  return {
    id: 'plan',
    targets: items.map((entry) => ({
      item_id: entry.id,
      name: entry.name,
      path: entry.path,
      mode: entry.risk === 'rebuild' ? 'trash' : 'permanent_delete',
      requires_confirmation: entry.requires_confirmation ?? false,
      expected_bytes: entry.size.allocated ?? entry.size.logical,
      risk: entry.risk,
    })),
    refused: [],
    expected_reclaim_bytes: 1024,
    risk: { safe_count: 0, rebuild_count: 1, manual_count: 0, safe_bytes: 0, rebuild_bytes: 1024, manual_bytes: 0 },
    requires_confirmation: false,
    expires_at: Math.floor(Date.now() / 1000) + 300,
    mode,
  };
}

describe('redesign interaction semantics', () => {
  it('exposes one keyboard tab stop and associates tabs with their panel', () => {
    const { body } = render(SegmentedTabs, { props: {
      tabs: [{ id: 'cleanup', label: 'Cleanup' }, { id: 'disks', label: 'Disks' }],
      activeTab: 'disks', onSelect: () => {}, panelId: 'storage-panel',
    } });
    expect(body.match(/tabindex="0"/g)).toHaveLength(1);
    expect(body.match(/aria-selected="true"/g)).toHaveLength(1);
    expect(body.match(/aria-controls="storage-panel"/g)).toHaveLength(2);
  });

  it('keeps the persisted singular disk route compatible with the Disks view', () => {
    expect(normalizeDashboardTab('disk')).toBe('disks');
    expect(normalizeDashboardTab('storage')).toBe('storage');
  });

  it('blocks resubmission while a batch action is pending', () => {
    const { body } = render(SelectionToolbar, { props: {
      selectedCount: 1, isActionLoading: true, onAction: () => {},
    } });
    expect(body).toContain('disabled');
    expect(body).toContain('Working…');
  });

  it('shows manual bytes and blocks generic cleanup for manual selections', () => {
    const { body } = render(SelectionToolbar, { props: {
      selectedCount: 1, selectedBytes: 0, manualBytes: 0, manualCount: 1, onAction: () => {},
    } });
    expect(body).toContain('1 item needs a separate action');
    expect(body).toContain('These items need a separate action');
    expect(body).toContain('disabled');
  });

  it('announces informational notices politely and errors urgently', () => {
    expect(render(InlineNotice, { props: { message: 'Refresh complete' } }).body).toContain('role="status"');
    expect(render(InlineNotice, { props: { variant: 'error', message: 'Refresh failed' } }).body).toContain('role="alert"');
  });

  it('keeps the reviewed paths and concise operation summary without per-item jargon', () => {
    const providerItem = {
      ...item,
      id: 'windows.recycle_bin',
      signature_id: 'windows.recycle_bin',
      name: 'Recycle Bin',
      risk: 'manual',
      cache_metadata: {
        provider: 'Windows',
        management_mode: 'tool_managed',
        artifact_kind: 'temporary',
        consequence: 'Items move to the Recycle Bin and can be restored until it is emptied.',
        size_semantics: 'physical_reclaimable',
        last_used_confidence: 'unknown',
      },
    } as ScanItem;
    const { body } = render(CleanupReviewDialog, { props: {
      plan: planFor([item, providerItem], 'mixed'),
      onCancel: () => {}, onConfirm: () => {},
    } });
    expect(body).toContain('Some items move to');
    expect(body).toContain('/fixture');
    expect(body).not.toContain('Rebuild items');
  });

  it('routes category cleanup through review before executing selected items', () => {
    const source = readFileSync(
      fileURLToPath(new URL('../routes/dashboard/CategoryDetailView.svelte', import.meta.url)),
      'utf8'
    );
    expect(source).toContain('CleanupReviewDialog');
    expect(source).toContain('confirmCleanup');
    expect(source).not.toContain('function cleanSelected() {\n    scanStore.cleanItems(categoryResult.items)');
  });

  it('explains rebuild consequences and blocks execution after scan invalidation', () => {
    const { body } = render(CleanupReviewDialog, { props: {
      plan: planFor([item]),
      disabled: true, onCancel: () => {}, onConfirm: () => {},
    } });
    expect(body).toContain('Build cache');
    expect(body).toContain('download or build again');
    expect(body).toContain('/fixture');
    expect(body).toContain('Move to');
    expect(body).toContain('This selection expired');
    const confirm = body.match(/<button[^>]*>[\s\S]*?<\/button>/g)?.find(button => button.includes('Clean items'));
    expect(confirm).toMatch(/<button[^>]*disabled/);
  });
});
