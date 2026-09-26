import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import type { ScanItem, ScanResult } from '../lib/models/types';
import { quickCleanupDetails } from '../lib/utils/quickCleanupDetails';
import Dialog from '../lib/components/QuickCleanupDetailsDialog.svelte';

function item(eligibility: NonNullable<ScanItem['disposition']>['eligibility'], bytes = 64): ScanItem {
  return { id: eligibility, signature_id: 'system.fixture', name: 'Fixture cache',
    category: 'system', risk: 'safe', path: '/tmp/zenith-fixture',
    size: { logical: bytes, allocated: bytes }, file_count: 1, description: '',
    is_selected: false, last_modified: null, exists: true, quality: 'fresh',
    incomplete_reason: null, disposition: { eligibility, reason: null, cleanable_bytes: bytes },
  };
}
function scan(items: ScanItem[]): ScanResult {
  return { scan_id: 'fixture', valid_for_seconds: 300, started_at: 1, finished_at: 2,
    categories: [{ category: 'system', display_name: 'System', items,
      total_bytes: 128, safe_bytes: 64, rebuild_bytes: 0, manual_bytes: 0 }],
    total_bytes: 128, safe_bytes: 64, rebuild_bytes: 0, manual_bytes: 0,
    quality: 'partial', incomplete_reasons: [],
    gaps: [{ kind: 'permission_denied', count: 2 }],
  };
}

describe('Quick cleanup explanations', () => {
  it('keeps reviewable and blocked observations out of automatic counts', () => {
    const details = quickCleanupDetails(scan([item('reviewable'), item('blocked'), item('recent')]), 0);
    expect(details.reviewCount).toBe(1);
    expect(details.excludedAutomaticCount).toBe(0);
    expect(details.blockedCount).toBe(1);
    expect(details.recentCount).toBe(1);
    expect(details.gaps).toEqual([{ kind: 'permission_denied', count: 2,
      label: 'Locations could not be read with the current permissions' }]);
  });
  it('counts only positive automatic candidates excluded from Quick Clean', () => {
    const inventory = scan([item('auto_cleanable'), item('auto_cleanable'), item('auto_cleanable', 0)]);
    expect(quickCleanupDetails(inventory, 1).excludedAutomaticCount).toBe(1);
  });
  it('explains the restriction and offers Storage and a deliberate rescan', () => {
    const body = render(Dialog, { props: { scan: scan([item('reviewable')]),
      quickEligibleCount: 0, busy: true, onClose() {}, onReview() {}, onRescan() {},
    } }).body;
    expect(body).toContain('No items currently qualify for Quick Clean');
    expect(body).toContain('1 item needs review in Storage');
    expect(body).toContain('current permissions (2)');
    expect(body).toContain('Open Storage');
    expect(body).toContain('Scan Again');
    expect(body).toContain('disabled');
    expect(body).not.toContain('/tmp/zenith-fixture');
  });
});
