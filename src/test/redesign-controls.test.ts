import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import SegmentedTabs from '../lib/components/SegmentedTabs.svelte';
import SelectionToolbar from '../lib/components/SelectionToolbar.svelte';
import InlineNotice from '../lib/components/InlineNotice.svelte';
import CleanupReviewDialog from '../lib/components/CleanupReviewDialog.svelte';
import type { ScanItem } from '../lib/models/types';
import { normalizeDashboardTab } from '../lib/utils/dashboardNavigation';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const item = {
  id: 'fixture', signature_id: 'fixture', name: 'Build cache', category: 'developer',
  risk: 'rebuild', size: { logical: 1024, allocated: 1024 }, path: '/fixture',
  file_count: 1, description: '', is_selected: true, exists: true, last_modified: null,
} as ScanItem;

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
    expect(body).toContain('1 Manual item');
    expect(body).toContain('Manual items require their dedicated management action');
    expect(body).toContain('disabled');
  });

  it('announces informational notices politely and errors urgently', () => {
    expect(render(InlineNotice, { props: { message: 'Refresh complete' } }).body).toContain('role="status"');
    expect(render(InlineNotice, { props: { variant: 'error', message: 'Refresh failed' } }).body).toContain('role="alert"');
  });

  it('shows the backend consequence statement verbatim for each item that carries one', () => {
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
      items: [item, providerItem], onCancel: () => {}, onConfirm: () => {},
    } });
    // The item's own consequence line, not a paraphrased warning.
    expect(body).toContain('Items move to the Recycle Bin and can be restored until it is emptied.');
    // Only the item that carries a consequence gets the meta line.
    expect(body.match(/text-meta/g)).toHaveLength(1);
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
      items: [item], disabled: true, onCancel: () => {}, onConfirm: () => {},
    } });
    expect(body).toContain('Build cache');
    expect(body).toContain('downloads or recompilation');
    expect(body).toContain('cannot be undone');
    expect(body).toContain('scan changed or expired');
    const confirm = body.match(/<button[^>]*>[\s\S]*?<\/button>/g)?.find(button => button.includes('Clean reviewed items'));
    expect(confirm).toMatch(/<button[^>]*disabled/);
  });
});
