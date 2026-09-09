import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import SegmentedTabs from '../lib/components/SegmentedTabs.svelte';
import SelectionToolbar from '../lib/components/SelectionToolbar.svelte';
import InlineNotice from '../lib/components/InlineNotice.svelte';
import CleanupReviewDialog from '../lib/components/CleanupReviewDialog.svelte';
import type { ScanItem } from '../lib/models/types';
import { normalizeDashboardTab } from '../lib/utils/dashboardNavigation';

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
